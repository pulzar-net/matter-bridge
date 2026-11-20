//! Certificates from a CA built on common PKI tooling (Python's
//! `cryptography`), in the Matter profile, converted to Matter TLV.

use matter_controller::x509::to_matter_tlv;
use rs_matter::cert::CertRef;
use rs_matter::tlv::TLVElement;

fn vectors() -> serde_json::Value {
    serde_json::from_str(include_str!("data/x509-matter-profile.json")).unwrap()
}

fn der(v: &serde_json::Value, field: &str) -> Vec<u8> {
    hex::decode(v[field].as_str().unwrap()).unwrap()
}

#[test]
fn a_root_and_its_intermediate_convert_and_name_the_fabric() {
    let v = vectors();
    let fabric_id = u64::from_str_radix(v["fabric_id"].as_str().unwrap(), 16).unwrap();
    let icac_id = u64::from_str_radix(v["icac_id"].as_str().unwrap(), 16).unwrap();

    let rcac = to_matter_tlv(&der(&v, "rcac_der")).expect("the root converts");
    let icac = to_matter_tlv(&der(&v, "icac_der")).expect("the intermediate converts");

    let rcac = CertRef::new(TLVElement::new(&rcac));
    let icac = CertRef::new(TLVElement::new(&icac));
    assert_eq!(rcac.get_fabric_id().unwrap(), fabric_id);
    assert_eq!(icac.get_fabric_id().unwrap(), fabric_id);
    assert_eq!(icac.get_ca_id().unwrap(), icac_id);
    assert!(rcac.is_self_signed().unwrap());
    assert!(!icac.is_self_signed().unwrap());
}
