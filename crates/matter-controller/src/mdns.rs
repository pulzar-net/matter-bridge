//! DNS-SD over mDNS with rs-matter's built-in responder and querier: it
//! advertises the controller as an operational node of its fabric and runs
//! the browses and resolves commissioning and CASE need.

use std::net::UdpSocket;

use rs_matter::crypto::Crypto;
use rs_matter::transport::network::mdns::builtin::{BuiltinMdns, Host};
use rs_matter::transport::network::mdns::{
    MDNS_IPV4_BROADCAST_ADDR, MDNS_IPV6_BROADCAST_ADDR, MDNS_SOCKET_DEFAULT_BIND_ADDR,
};
use rs_matter::transport::network::{Ipv4Addr, Ipv6Addr};
use rs_matter::Matter;
use socket2::{Domain, Protocol, Socket, Type};

use crate::Error;

/// The interface mDNS runs on: its name, index, an IPv4 address (unspecified
/// if none) and every IPv6 address.
struct Interface {
    name: String,
    index: u32,
    ipv4: std::net::Ipv4Addr,
    ipv6: Vec<std::net::Ipv6Addr>,
}

/// Pick the interface named `wanted`, or else the first non-loopback one
/// with a link-local IPv6 address (the LAN, rather than a container bridge),
/// or any IPv6 address.
fn pick_interface(wanted: Option<&str>) -> Result<Interface, Error> {
    let all = if_addrs::get_if_addrs()?;
    let candidates: Vec<_> = all
        .iter()
        .filter(|ia| !ia.is_loopback())
        .filter(|ia| wanted.is_none_or(|w| ia.name == w))
        .collect();

    let has_v6 = |pred: &dyn Fn(&std::net::Ipv6Addr) -> bool| {
        candidates.iter().find_map(|ia| match &ia.addr {
            if_addrs::IfAddr::V6(v6) if pred(&v6.ip) => Some(ia.name.clone()),
            _ => None,
        })
    };
    let name = has_v6(&|ip| ip.is_unicast_link_local())
        .or_else(|| has_v6(&|_| true))
        .or_else(|| candidates.first().map(|ia| ia.name.clone()))
        .ok_or_else(|| {
            Error::Invalid(match wanted {
                Some(w) => format!("no network interface {w}"),
                None => "no network interface for mDNS".into(),
            })
        })?;

    let mut iface = Interface {
        index: 0,
        ipv4: std::net::Ipv4Addr::UNSPECIFIED,
        ipv6: Vec::new(),
        name,
    };
    for ia in all.iter().filter(|ia| ia.name == iface.name) {
        iface.index = ia.index.unwrap_or(iface.index);
        match &ia.addr {
            if_addrs::IfAddr::V4(v4) if iface.ipv4.is_unspecified() => iface.ipv4 = v4.ip,
            if_addrs::IfAddr::V6(v6) if !iface.ipv6.contains(&v6.ip) => iface.ipv6.push(v6.ip),
            _ => {}
        }
    }
    Ok(iface)
}

/// Run mDNS until an error.
pub(crate) async fn run<C: Crypto>(
    matter: &Matter<'_>,
    crypto: C,
    interface: Option<&str>,
) -> Result<(), Error> {
    let iface = pick_interface(interface)?;
    log::info!("mDNS on {} ({}, {:?})", iface.name, iface.ipv4, iface.ipv6);

    let socket = Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP))?;
    socket.set_reuse_address(true)?;
    #[cfg(unix)]
    socket.set_reuse_port(true)?;
    socket.set_only_v6(false)?;
    socket.bind(&MDNS_SOCKET_DEFAULT_BIND_ADDR.into())?;
    let socket = async_io::Async::<UdpSocket>::new_nonblocking(socket.into())?;
    socket
        .get_ref()
        .join_multicast_v6(&MDNS_IPV6_BROADCAST_ADDR, iface.index)?;
    if !iface.ipv4.is_unspecified() {
        socket
            .get_ref()
            .join_multicast_v4(&MDNS_IPV4_BROADCAST_ADDR, &iface.ipv4)?;
    }

    // The host name is per-boot random, as the spec suggests for a node
    // without a MAC address to hand.
    let hostname = format!("{:012X}", rand::random::<u64>() & 0xFFFF_FFFF_FFFF);
    let ipv6: Vec<Ipv6Addr> = iface.ipv6.iter().map(|ip| ip.octets().into()).collect();
    let ipv4: Ipv4Addr = iface.ipv4.octets().into();

    BuiltinMdns::new()
        .run(
            &socket,
            &socket,
            &Host {
                hostname: &hostname,
                ip: ipv4,
                ipv6: &ipv6,
            },
            (!iface.ipv4.is_unspecified()).then_some(ipv4),
            Some(iface.index),
            matter,
            crypto,
        )
        .await?;
    Ok(())
}
