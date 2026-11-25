//! Operational network credentials for devices being commissioned.
//!
//! A Thread network is given to a device as its active operational dataset:
//! MeshCoP TLVs (one byte of type, one byte of length, the value), as a
//! border router hands them out. The controller sends the dataset as it is;
//! it only reads the extended PAN id, which names the network in
//! `ConnectNetwork` and is the one part of the dataset safe to log.

use crate::Error;

/// The largest dataset Matter's `AddOrUpdateThreadNetwork` takes.
pub const MAX_DATASET_LEN: usize = 254;

const TLV_EXTENDED_PAN_ID: u8 = 2;
const TLV_NETWORK_KEY: u8 = 5;

/// Walk the dataset's TLVs, checking each fits, and return the value of the
/// first with type `wanted`.
fn find(dataset: &[u8], wanted: u8) -> Result<Option<&[u8]>, Error> {
    let invalid = |why: &str| Error::Invalid(format!("Thread dataset: {why}"));
    if dataset.is_empty() || dataset.len() > MAX_DATASET_LEN {
        return Err(invalid("must be 1 to 254 bytes"));
    }
    let mut found = None;
    let mut rest = dataset;
    while let [ty, len, tail @ ..] = rest {
        let len = usize::from(*len);
        let Some((value, next)) = tail.split_at_checked(len) else {
            return Err(invalid("a TLV runs past the end"));
        };
        if *ty == wanted && found.is_none() {
            found = Some(value);
        }
        rest = next;
    }
    if !rest.is_empty() {
        return Err(invalid("trailing byte"));
    }
    Ok(found)
}

/// The extended PAN id of a Thread active operational dataset, after
/// checking the dataset is well-formed and carries a network key.
pub fn thread_extended_pan_id(dataset: &[u8]) -> Result<[u8; 8], Error> {
    if find(dataset, TLV_NETWORK_KEY)?.is_none_or(|k| k.len() != 16) {
        return Err(Error::Invalid(
            "Thread dataset: no 16-byte network key".into(),
        ));
    }
    find(dataset, TLV_EXTENDED_PAN_ID)?
        .and_then(|v| <[u8; 8]>::try_from(v).ok())
        .ok_or_else(|| Error::Invalid("Thread dataset: no 8-byte extended PAN id".into()))
}

/// Parse a dataset given as hex (whitespace ignored), as `ot-ctl dataset
/// active -x` and a border router's REST API give it.
pub fn parse_thread_dataset_hex(text: &str) -> Result<Vec<u8>, Error> {
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let dataset =
        hex::decode(&compact).map_err(|_| Error::Invalid("Thread dataset: not hex".into()))?;
    thread_extended_pan_id(&dataset)?;
    Ok(dataset)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An OpenThread active dataset (the documentation's example network).
    const DATASET: &str = "0e080000000000010000000300001435060004001fffe00208dead00beef00cafe0708fddead00beef000005101234567890abcdef1234567890abcdef030a4f70656e546872656164010212340410a6cb6ea0e9e9bc4d1a7dfe9a6b4fd0190c0402a0f7f8";

    #[test]
    fn reads_the_extended_pan_id() {
        let dataset = parse_thread_dataset_hex(DATASET).unwrap();
        assert_eq!(
            thread_extended_pan_id(&dataset).unwrap(),
            [0xde, 0xad, 0x00, 0xbe, 0xef, 0x00, 0xca, 0xfe]
        );
    }

    #[test]
    fn refuses_malformed_datasets() {
        let good = hex::decode(DATASET).unwrap();
        assert!(thread_extended_pan_id(&good[..good.len() - 1]).is_err());
        assert!(thread_extended_pan_id(&[]).is_err());
        assert!(thread_extended_pan_id(&[2, 8, 1, 2, 3]).is_err());
        assert!(
            thread_extended_pan_id(&[2, 8, 0, 0, 0, 0, 0, 0, 0, 0]).is_err(),
            "no key"
        );
        assert!(thread_extended_pan_id(&vec![0u8; 300]).is_err());
        assert!(parse_thread_dataset_hex("zz").is_err());
    }

    #[test]
    fn debug_never_shows_secrets() {
        let networks = crate::Networks {
            thread_dataset: Some(hex::decode(DATASET).unwrap()),
            wifi: Some(crate::WiFi {
                ssid: b"home".to_vec(),
                credentials: b"secret".to_vec(),
            }),
        };
        let shown = format!("{networks:?}");
        assert!(
            shown.contains("dead00beef00cafe") && shown.contains("home"),
            "{shown}"
        );
        assert!(
            !shown.contains("secret") && !shown.contains("1234567890abcdef"),
            "{shown}"
        );
    }
}
