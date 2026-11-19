//! Onboarding payloads: the QR code (`MT:...`) and the manual pairing code a
//! device carries, parsed with rs-matter's parsers.

use rs_matter::pairing::qr::{CommFlowType, QrPayload};
use rs_matter::pairing::DiscoveryCapabilities;
use rs_matter::transport::network::mdns::CommissionableFilter;
use serde::Serialize;

use crate::Error;

/// Which discriminator an onboarding payload carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Discriminator {
    /// The full 12 bits, from a QR code.
    Long(u16),
    /// The upper 4 bits, from a manual pairing code.
    Short(u8),
}

/// How a device can be found for commissioning, from its QR code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Capabilities {
    /// Advertising over Bluetooth LE.
    pub ble: bool,
    /// Already on the IP network.
    pub on_network: bool,
    /// Running a Wi-Fi soft access point.
    pub soft_ap: bool,
}

/// What an onboarding payload says about a device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Onboarding {
    /// The setup passcode.
    #[serde(skip)]
    pub passcode: u32,
    /// The discriminator.
    pub discriminator: Discriminator,
    /// Vendor id, if the payload has it.
    pub vendor_id: Option<u16>,
    /// Product id, if the payload has it.
    pub product_id: Option<u16>,
    /// How the device can be found; `None` for a manual code, which does not
    /// say, so every way is tried.
    pub capabilities: Option<Capabilities>,
    /// Whether the device needs a custom or user-intent commissioning flow
    /// (a button press, a vendor app) before it can be commissioned.
    pub custom_flow: bool,
}

impl Onboarding {
    /// Parse a QR code payload (`MT:...`) or an 11- or 21-digit manual code
    /// (separators allowed).
    pub fn parse(code: &str) -> Result<Self, Error> {
        let code = code.trim();
        if code.starts_with("MT:") {
            let mut buf = [0u8; 256];
            let qr = QrPayload::parse(code, &mut buf)
                .map_err(|e| Error::Onboarding(format!("not a valid Matter QR code: {e:?}")))?;
            let caps = qr.discovery_capabilities();
            Ok(Self {
                passcode: qr.passcode(),
                discriminator: Discriminator::Long(qr.discriminator()),
                vendor_id: Some(qr.vid()),
                product_id: Some(qr.pid()),
                capabilities: Some(Capabilities {
                    ble: caps.contains(DiscoveryCapabilities::BLE),
                    on_network: caps.contains(DiscoveryCapabilities::IP),
                    soft_ap: caps.contains(DiscoveryCapabilities::SOFT_AP),
                }),
                custom_flow: !matches!(qr.comm_flow(), CommFlowType::Standard),
            })
        } else {
            let manual = QrPayload::parse_pairing_code(code).map_err(|e| {
                Error::Onboarding(format!("not a valid manual pairing code: {e:?}"))
            })?;
            let (vendor_id, product_id) = match manual.vid_pid() {
                Some((v, p)) => (Some(v), Some(p)),
                None => (None, None),
            };
            Ok(Self {
                passcode: manual.passcode(),
                discriminator: Discriminator::Short(manual.short_discriminator()),
                vendor_id,
                product_id,
                capabilities: None,
                custom_flow: !matches!(manual.comm_flow(), Some(CommFlowType::Standard) | None),
            })
        }
    }

    /// The mDNS filter that finds this device among commissionable nodes.
    pub fn filter(&self) -> CommissionableFilter {
        match self.discriminator {
            Discriminator::Long(d) => CommissionableFilter {
                discriminator: Some(d),
                ..Default::default()
            },
            Discriminator::Short(d) => CommissionableFilter {
                short_discriminator: Some(d),
                ..Default::default()
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The CSA's test device payloads (the C++ SDK's defaults: passcode
    // 20202021, discriminator 3840, vendor 0xFFF1, product 0x8000).
    #[test]
    fn qr_code() {
        let o = Onboarding::parse("MT:-24J0AFN00KA0648G00").unwrap();
        assert_eq!(o.passcode, 20202021);
        assert_eq!(o.discriminator, Discriminator::Long(3840));
        assert_eq!(o.vendor_id, Some(0xFFF1));
        assert!(o.capabilities.unwrap().on_network || o.capabilities.unwrap().ble);
    }

    #[test]
    fn manual_code() {
        let o = Onboarding::parse("3497-011-2332").unwrap();
        assert_eq!(o.passcode, 20202021);
        assert_eq!(o.discriminator, Discriminator::Short(15));
        assert!(!o.custom_flow);
        assert_eq!(o.filter().short_discriminator, Some(15));
    }

    #[test]
    fn rejects_garbage() {
        for code in ["", "MT:", "MT:!!!", "1234", "34970112333", "hello"] {
            assert!(Onboarding::parse(code).is_err(), "{code}");
        }
    }
}
