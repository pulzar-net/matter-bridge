//! Commissioning over Bluetooth LE: finding a device by its Matter
//! advertisement and carrying rs-matter's BTP over GATT, with the GATT
//! central from bluetooth-bridge's `bluetooth-host`.
//!
//! rs-matter drives BTP in the central (initiator) role; this module is the
//! GATT half: it writes what `Btp::process_outgoing` produces to the C1
//! characteristic and feeds C2 indications to `Btp::process_incoming`. PASE
//! then runs over `Address::Btp`, and the rest of commissioning over IP once
//! the device has joined its operational network.

use std::time::Duration;

use bluetooth_host::adv::Uuid;
use bluetooth_host::{ConnEvent, ConnectParams, Connection, Dedup, Host, HostEvent, ScanConfig};
use rs_matter::transport::network::btp::{AdvData, Btp};
use rs_matter::transport::network::mdns::CommissionableFilter;
use rs_matter::transport::network::BtAddr;
use tokio::sync::{broadcast, oneshot};

use crate::Error;

/// The Matter BLE service (16-bit UUID).
const MATTER_SERVICE: u16 = 0xFFF6;
/// C1, the characteristic the central writes BTP packets to.
const C1: [u8; 16] = 0x18EE2EF5263D4559959F4F9C429F9D11u128.to_be_bytes();
/// C2, the characteristic the peripheral indicates BTP packets on.
const C2: [u8; 16] = 0x18EE2EF5263D4559959F4F9C429F9D12u128.to_be_bytes();
/// The ATT MTU asked for; BTP segments to fit what is agreed.
const ATT_MTU: u16 = 247;

fn ble_err(e: impl std::fmt::Display) -> Error {
    Error::Bluetooth(e.to_string())
}

/// The Bluetooth side of the controller: the host on one controller and the
/// BTP state rs-matter's transport sends `Address::Btp` packets through.
pub(crate) struct Ble {
    pub host: Host,
    pub btp: &'static Btp,
}

impl Ble {
    pub async fn open(spec: &str, btp: &'static Btp) -> Result<Self, Error> {
        let spec: bluetooth_host::TransportSpec = spec
            .parse()
            .map_err(|e| Error::Invalid(format!("bluetooth: {e}")))?;
        let host = Host::open(&spec, bluetooth_host::HostConfig::default())
            .await
            .map_err(ble_err)?;
        log::info!("Bluetooth on {spec}");
        Ok(Self { host, btp })
    }

    /// Scan for a commissionable device whose advertisement matches `filter`.
    pub async fn find(
        &self,
        filter: &CommissionableFilter,
        timeout: Duration,
    ) -> Result<bluetooth_host::Address, Error> {
        let mut events = self.host.events();
        self.host
            .start_scan(ScanConfig {
                active: false,
                dedup: Dedup::Changes,
                ..Default::default()
            })
            .await
            .map_err(ble_err)?;
        let found = tokio::time::timeout(timeout, async {
            loop {
                match events.recv().await {
                    Ok(HostEvent::Advertisement(report)) => {
                        let adv = report.advertisement();
                        let matter = adv
                            .service_data_for(Uuid::Uuid16(MATTER_SERVICE))
                            .and_then(AdvData::parse_service_data);
                        if matter.is_some_and(|m| m.matches(filter)) {
                            return Ok(report.address);
                        }
                    }
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => return Err(Error::Stopped),
                }
            }
        })
        .await;
        let _ = self.host.stop_scan().await;
        found.map_err(|_| {
            Error::Timeout("no commissionable device advertised over Bluetooth".into())
        })?
    }

    /// Connect to `peer` and carry BTP until [`Link::close`]; the caller then
    /// reaches the device at the returned `Address::Btp`.
    pub async fn connect(&self, peer: bluetooth_host::Address) -> Result<(BtAddr, Link), Error> {
        let conn = self
            .host
            .connect(
                peer,
                &ConnectParams {
                    timeout_ms: 10_000,
                    ..Default::default()
                },
            )
            .await
            .map_err(ble_err)?;
        let addr = BtAddr(peer.bytes);
        let btp = self.btp;
        btp.reset();
        let (stop_tx, stop_rx) = oneshot::channel();
        let (ready_tx, ready_rx) = oneshot::channel();
        let task = tokio::task::spawn_local(async move {
            let result = pump(&conn, addr, btp, ready_tx, stop_rx).await;
            let _ = conn.disconnect().await;
            btp.reset();
            if let Err(e) = &result {
                log::warn!("Bluetooth link to {addr}: {e}");
            }
        });
        // The link is up once the BTP handshake has gone out and C2 is
        // subscribed; PASE starts then.
        match ready_rx.await {
            Ok(Ok(())) => Ok((
                addr,
                Link {
                    stop: Some(stop_tx),
                    task,
                },
            )),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(Error::Bluetooth(
                "the link closed while it was set up".into(),
            )),
        }
    }
}

/// A Bluetooth link carrying BTP; closed when commissioning is done.
pub(crate) struct Link {
    stop: Option<oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<()>,
}

impl Link {
    /// Disconnect and wait until the link is down.
    pub async fn close(mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        let _ = (&mut self.task).await;
    }
}

async fn pump(
    conn: &Connection,
    addr: BtAddr,
    btp: &Btp,
    ready: oneshot::Sender<Result<(), Error>>,
    stop: oneshot::Receiver<()>,
) -> Result<(), Error> {
    let gatt = conn.gatt();
    let setup = async {
        match gatt.exchange_mtu(ATT_MTU).await {
            Ok(mtu) => log::debug!("ATT MTU {mtu}"),
            Err(e) => log::warn!("ATT MTU exchange: {e}"),
        }
        let service = gatt
            .services_by_uuid(Uuid::Uuid16(MATTER_SERVICE))
            .await
            .map_err(ble_err)?
            .into_iter()
            .next()
            .ok_or_else(|| Error::Bluetooth("the device has no Matter GATT service".into()))?;
        log::debug!("Matter GATT service at {}..{}", service.start, service.end);
        let chars = gatt.characteristics(&service).await.map_err(ble_err)?;
        let find = |uuid: [u8; 16]| {
            chars
                .iter()
                .find(|c| c.uuid == Uuid::Uuid128(uuid))
                .cloned()
                .ok_or_else(|| Error::Bluetooth("the Matter GATT service lacks C1 or C2".into()))
        };
        Ok::<_, Error>((find(C1)?, find(C2)?))
    };
    let (c1, c2) = match setup.await {
        Ok(chars) => chars,
        Err(e) => {
            let _ = ready.send(Err(Error::Bluetooth(e.to_string())));
            return Err(e);
        }
    };
    let gatt_mtu = Some(conn.att_mtu());

    // Listen before subscribing, so the handshake response is not missed.
    let mut events = conn.events();

    // As the C++ SDK's peripherals want it: the handshake request on C1
    // first, then the C2 subscription, which triggers the response.
    btp.set_initiator(true);
    let mut buf = [0u8; 512];
    let handshake = async {
        let len = btp.process_outgoing(gatt_mtu, &mut buf)?;
        if len == 0 {
            return Err(Error::Bluetooth("BTP had no handshake to send".into()));
        }
        gatt.write(c1.value_handle, &buf[..len])
            .await
            .map_err(ble_err)?;
        gatt.subscribe(&c2, false, true).await.map_err(ble_err)?;
        Ok(())
    };
    if let Err(e) = handshake.await {
        let _ = ready.send(Err(Error::Bluetooth(e.to_string())));
        return Err(e);
    }
    log::debug!("BTP handshake sent, C2 subscribed");
    let _ = ready.send(Ok(()));

    let incoming = async {
        loop {
            match events.recv().await {
                Ok(ConnEvent::Notification { handle, value, .. })
                    if handle == c2.value_handle && !value.is_empty() =>
                {
                    btp.process_incoming(gatt_mtu, addr, &value)?;
                }
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => return Ok::<_, Error>(()),
            }
        }
    };
    let outgoing = async {
        let mut out = [0u8; 512];
        loop {
            loop {
                let len = btp.process_outgoing(gatt_mtu, &mut out)?;
                if len == 0 {
                    break;
                }
                gatt.write(c1.value_handle, &out[..len])
                    .await
                    .map_err(ble_err)?;
            }
            btp.wait_outgoing().await;
        }
    };

    tokio::select! {
        r = incoming => r,
        r = outgoing => r,
        _ = conn.disconnected() => Ok(()),
        () = btp.wait_timeout() => Err(Error::Timeout("the BTP session went idle".into())),
        _ = stop => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn characteristic_uuids() {
        assert_eq!(
            Uuid::Uuid128(C1),
            Uuid::parse("18ee2ef5-263d-4559-959f-4f9c429f9d11").unwrap()
        );
        assert_eq!(
            Uuid::Uuid128(C2),
            Uuid::parse("18ee2ef5-263d-4559-959f-4f9c429f9d12").unwrap()
        );
    }
}
