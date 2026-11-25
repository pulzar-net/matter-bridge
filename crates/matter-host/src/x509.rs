//! X.509 DER certificates in the Matter certificate profile, as a CA built on
//! common PKI tooling issues them, converted to the Matter TLV encoding that
//! Matter nodes exchange (Matter Core spec, "Certificate Encoding").
//!
//! A Matter TLV certificate carries the same fields as the X.509 one and the
//! same signature, which is over the X.509 encoding: a node verifying it
//! rebuilds the DER from the TLV. So every conversion is checked by rebuilding
//! the DER with rs-matter and comparing it with the input byte for byte. A
//! certificate outside the profile (another string type, an extension
//! encoded otherwise) is refused here rather than installed with a signature
//! no node would accept.

use der::{Decode, Encode, Tagged as _};
use rs_matter::cert::CertRef;
use rs_matter::tlv::{TLVElement, TLVTag, TLVWrite};
use rs_matter::utils::storage::WriteBuf;
use x509_cert::ext::pkix::{
    AuthorityKeyIdentifier, BasicConstraints, ExtendedKeyUsage, KeyUsage, SubjectKeyIdentifier,
};
use x509_cert::name::Name;
use x509_cert::time::Time;
use x509_cert::Certificate;

use crate::Error;

/// Seconds from the Unix epoch to the Matter epoch (2000-01-01T00:00:00Z).
const MATTER_EPOCH: u64 = 946_684_800;
/// `99991231235959Z`: "no well-defined expiration date", 0 in Matter TLV.
const NO_EXPIRY: u64 = 253_402_300_799;

fn invalid(what: impl std::fmt::Display) -> Error {
    Error::Invalid(format!("certificate: {what}"))
}

fn tlv(e: rs_matter::error::Error) -> Error {
    invalid(format!("encoding: {e:?}"))
}

/// Convert an X.509 DER certificate in the Matter profile (an RCAC, ICAC or
/// NOC) to Matter TLV.
pub fn to_matter_tlv(der: &[u8]) -> Result<Vec<u8>, Error> {
    let cert = Certificate::from_der(der).map_err(|e| invalid(format!("not X.509 DER: {e}")))?;
    let tbs = cert.tbs_certificate();

    const ECDSA_WITH_SHA256: &str = "1.2.840.10045.4.3.2";
    const EC_PUBLIC_KEY: &str = "1.2.840.10045.2.1";
    const PRIME256V1: &str = "1.2.840.10045.3.1.7";
    if cert.signature_algorithm().oid.to_string() != ECDSA_WITH_SHA256 {
        return Err(invalid("not signed with ECDSA with SHA-256"));
    }
    let spki = tbs.subject_public_key_info();
    let curve = spki
        .algorithm
        .parameters
        .as_ref()
        .and_then(|p| p.decode_as::<der::asn1::ObjectIdentifier>().ok())
        .map(|o| o.to_string());
    if spki.algorithm.oid.to_string() != EC_PUBLIC_KEY || curve.as_deref() != Some(PRIME256V1) {
        return Err(invalid("not a P-256 public key"));
    }
    let public_key = spki
        .subject_public_key
        .as_bytes()
        .filter(|k| k.len() == 65)
        .ok_or_else(|| invalid("not an uncompressed P-256 public key"))?;

    let mut buf = vec![0u8; 1024];
    let mut w = WriteBuf::new(&mut buf);
    let ctx = TLVTag::Context;
    w.start_struct(&TLVTag::Anonymous).map_err(tlv)?;
    w.str(&ctx(1), tbs.serial_number().as_bytes())
        .map_err(tlv)?;
    w.u8(&ctx(2), 1).map_err(tlv)?; // ecdsa-with-SHA256
    name(&mut w, 3, tbs.issuer())?;
    w.u32(&ctx(4), matter_time(&tbs.validity().not_before)?)
        .map_err(tlv)?;
    w.u32(&ctx(5), matter_time(&tbs.validity().not_after)?)
        .map_err(tlv)?;
    name(&mut w, 6, tbs.subject())?;
    w.u8(&ctx(7), 1).map_err(tlv)?; // EC public key
    w.u8(&ctx(8), 1).map_err(tlv)?; // prime256v1
    w.str(&ctx(9), public_key).map_err(tlv)?;

    w.start_list(&ctx(10)).map_err(tlv)?;
    for ext in tbs.extensions().into_iter().flatten() {
        let value = ext.extn_value.as_bytes();
        match ext.extn_id.to_string().as_str() {
            "2.5.29.19" => {
                let bc = BasicConstraints::from_der(value).map_err(invalid)?;
                w.start_struct(&ctx(1)).map_err(tlv)?;
                w.bool(&ctx(1), bc.ca).map_err(tlv)?;
                if let Some(len) = bc.path_len_constraint {
                    w.u8(&ctx(2), len).map_err(tlv)?;
                }
                w.end_container().map_err(tlv)?;
            }
            "2.5.29.15" => {
                // Matter's key usage bits are X.509's, in the same order.
                let ku = KeyUsage::from_der(value).map_err(invalid)?;
                w.u16(&ctx(2), ku.0.bits()).map_err(tlv)?;
            }
            "2.5.29.37" => {
                let eku = ExtendedKeyUsage::from_der(value).map_err(invalid)?;
                w.start_array(&ctx(3)).map_err(tlv)?;
                for oid in eku.0 {
                    let id = match oid.to_string().as_str() {
                        "1.3.6.1.5.5.7.3.1" => 1, // serverAuth
                        "1.3.6.1.5.5.7.3.2" => 2, // clientAuth
                        "1.3.6.1.5.5.7.3.3" => 3, // codeSigning
                        "1.3.6.1.5.5.7.3.4" => 4, // emailProtection
                        "1.3.6.1.5.5.7.3.8" => 5, // timeStamping
                        "1.3.6.1.5.5.7.3.9" => 6, // OCSPSigning
                        other => return Err(invalid(format!("extended key usage {other}"))),
                    };
                    w.u8(&TLVTag::Anonymous, id).map_err(tlv)?;
                }
                w.end_container().map_err(tlv)?;
            }
            "2.5.29.14" => {
                let skid = SubjectKeyIdentifier::from_der(value).map_err(invalid)?;
                w.str(&ctx(4), skid.0.as_bytes()).map_err(tlv)?;
            }
            "2.5.29.35" => {
                let akid = AuthorityKeyIdentifier::from_der(value).map_err(invalid)?;
                let id = akid
                    .key_identifier
                    .ok_or_else(|| invalid("authority key identifier without a key identifier"))?;
                w.str(&ctx(5), id.as_bytes()).map_err(tlv)?;
            }
            _ => {
                let whole = ext.to_der().map_err(invalid)?;
                w.str(&ctx(6), &whole).map_err(tlv)?;
            }
        }
    }
    w.end_container().map_err(tlv)?;

    let signature = cert
        .signature()
        .as_bytes()
        .ok_or_else(|| invalid("signature"))?;
    let signature = p256::ecdsa::Signature::from_der(signature)
        .map_err(|_| invalid("not an ECDSA P-256 signature"))?;
    w.str(&ctx(11), &signature.to_bytes()).map_err(tlv)?;
    w.end_container().map_err(tlv)?;
    let len = w.get_tail();
    buf.truncate(len);

    // A node rebuilds the signed DER from the TLV: it must be the DER that
    // was signed.
    let signed = tbs.to_der().map_err(invalid)?;
    let mut rebuilt = vec![0u8; signed.len() + 64];
    let n = CertRef::new(TLVElement::new(&buf))
        .as_asn1(&mut rebuilt)
        .map_err(|e| invalid(format!("rebuilding the DER: {e:?}")))?;
    if rebuilt[..n] != signed[..] {
        return Err(invalid(
            "not in the Matter certificate profile: as Matter TLV it would not verify",
        ));
    }
    Ok(buf)
}

fn matter_time(t: &Time) -> Result<u32, Error> {
    let unix = t.to_unix_duration().as_secs();
    if unix == NO_EXPIRY {
        return Ok(0);
    }
    unix.checked_sub(MATTER_EPOCH)
        .and_then(|s| u32::try_from(s).ok())
        .ok_or_else(|| invalid("a validity date outside 2000 to 2135"))
}

fn name(w: &mut WriteBuf<'_>, tag: u8, name: &Name) -> Result<(), Error> {
    w.start_list(&TLVTag::Context(tag)).map_err(tlv)?;
    for atv in name.iter() {
        let value = &atv.value;
        let text = || {
            std::str::from_utf8(value.value())
                .map_err(|_| invalid("a name attribute that is not text"))
        };
        let hex = |len: usize| -> Result<u64, Error> {
            let t = text()?;
            if t.len() != len || value.tag() != der::Tag::Utf8String {
                return Err(invalid(format!(
                    "a Matter id that is not {len} hex digits in a UTF8String"
                )));
            }
            u64::from_str_radix(t, 16).map_err(|_| invalid("a Matter id that is not hex"))
        };
        let ctx = TLVTag::Context;
        match atv.oid.to_string().as_str() {
            "1.3.6.1.4.1.37244.1.1" => w.u64(&ctx(17), hex(16)?).map_err(tlv)?,
            "1.3.6.1.4.1.37244.1.2" => w.u64(&ctx(18), hex(16)?).map_err(tlv)?,
            "1.3.6.1.4.1.37244.1.3" => w.u64(&ctx(19), hex(16)?).map_err(tlv)?,
            "1.3.6.1.4.1.37244.1.4" => w.u64(&ctx(20), hex(16)?).map_err(tlv)?,
            "1.3.6.1.4.1.37244.1.5" => w.u64(&ctx(21), hex(16)?).map_err(tlv)?,
            "1.3.6.1.4.1.37244.1.6" => w.u32(&ctx(22), hex(8)? as u32).map_err(tlv)?,
            oid => {
                let base = match oid {
                    "2.5.4.3" => 1,                     // commonName
                    "2.5.4.4" => 2,                     // surname
                    "2.5.4.5" => 3,                     // serialNumber
                    "2.5.4.6" => 4,                     // countryName
                    "2.5.4.7" => 5,                     // localityName
                    "2.5.4.8" => 6,                     // stateOrProvinceName
                    "2.5.4.10" => 7,                    // organizationName
                    "2.5.4.11" => 8,                    // organizationalUnitName
                    "2.5.4.12" => 9,                    // title
                    "2.5.4.41" => 10,                   // name
                    "2.5.4.42" => 11,                   // givenName
                    "2.5.4.43" => 12,                   // initials
                    "2.5.4.44" => 13,                   // generationQualifier
                    "2.5.4.46" => 14,                   // dnQualifier
                    "2.5.4.65" => 15,                   // pseudonym
                    "0.9.2342.19200300.100.1.25" => 16, // domainComponent
                    other => return Err(invalid(format!("name attribute {other}"))),
                };
                // A PrintableString attribute has its own tags, 0x80 above.
                let tag = match value.tag() {
                    der::Tag::PrintableString => base | 0x80,
                    der::Tag::Utf8String | der::Tag::Ia5String => base,
                    other => return Err(invalid(format!("a name attribute as {other}"))),
                };
                w.utf8(&ctx(tag), text()?).map_err(tlv)?;
            }
        }
    }
    w.end_container().map_err(tlv)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_what_is_not_a_certificate() {
        assert!(to_matter_tlv(&[]).is_err());
        assert!(to_matter_tlv(&[0x30, 0x03, 0x02, 0x01, 0x01]).is_err());
        assert!(to_matter_tlv(&[0x15, 0x18]).is_err());
    }

    /// A certificate rs-matter made (TLV), as X.509 DER, back to the same TLV.
    #[test]
    fn round_trips_a_matter_certificate() {
        use rs_matter::cert::gen::VALID_FOREVER;
        use rs_matter::cert::MAX_CERT_TLV_AND_ASN1_LEN;
        use rs_matter::crypto::CanonPkcSecretKeyRef;
        use rs_matter::onboard::cac::RcacGenerator;

        static KEY: [u8; 32] = [3; 32];
        let crypto =
            rs_matter::crypto::default_crypto(rand::rng(), CanonPkcSecretKeyRef::new(&KEY));
        let mut buf = vec![0u8; MAX_CERT_TLV_AND_ASN1_LEN];
        let mut gen = RcacGenerator::new(&mut buf);
        let (_, rcac) = gen.generate(&crypto, 0x1234, VALID_FOREVER).unwrap();
        let rcac = rcac.to_vec();

        // Rebuild the whole X.509 certificate: the TBS rs-matter encodes,
        // the algorithm, and the signature as DER.
        let cert = CertRef::new(TLVElement::new(&rcac));
        let mut tbs = vec![0u8; 1024];
        let n = cert.as_asn1(&mut tbs).unwrap();
        let tbs = &tbs[..n];
        let raw_sig = TLVElement::new(&rcac)
            .structure()
            .unwrap()
            .ctx(11)
            .unwrap()
            .str()
            .unwrap()
            .to_vec();
        let sig = p256::ecdsa::Signature::from_slice(&raw_sig)
            .unwrap()
            .to_der();
        let algo = [
            0x30, 0x0a, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02,
        ];
        let sig_bits = der::asn1::BitString::from_bytes(sig.as_bytes())
            .unwrap()
            .to_der()
            .unwrap();
        let body: Vec<u8> = [tbs, &algo[..], &sig_bits[..]].concat();
        let mut der = vec![0x30, 0x82, (body.len() >> 8) as u8, body.len() as u8];
        der.extend_from_slice(&body);

        assert_eq!(to_matter_tlv(&der).unwrap(), rcac);
    }
}
