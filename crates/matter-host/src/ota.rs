//! The OTA provider: the controller offers the nodes it commissioned the
//! Matter OTA image files in a directory, and serves them over BDX.
//!
//! rs-matter's OTA Provider cluster answers a node's `QueryImage` and its BDX
//! handler streams the image; this module tells them which image a node gets
//! (the newest for its vendor and product that applies on top of its version)
//! and where its bytes are. A file's name is its BDX file designator.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use rs_matter::clusters::ota_prov::{
    OtaApplyOutcome, OtaImageMeta, OtaImages, OtaImagesRegistry, OtaQueryOutcome, OtaRequestorId,
};
use rs_matter::error::{Error as MatterError, ErrorCode};
use rs_matter::ota::OtaImageHeader;
use serde::Serialize;

use crate::controller::EventBus;
use crate::{Event, OtaStep};

/// The most of a file read to find its header; a valid header is far
/// smaller.
const MAX_HEADER: u64 = 1024;

/// The longest file name served: the designator, version string and update
/// token must fit the 192 bytes rs-matter lends the query.
const MAX_NAME: usize = 96;

/// An OTA image file the controller offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OtaImage {
    /// The file's name in the OTA directory.
    pub file: String,
    /// The vendor it is for; 0 for any.
    pub vendor_id: u16,
    /// The product it is for; 0 for any.
    pub product_id: u16,
    /// The software version it installs.
    pub version: u32,
    /// The displayable version.
    pub version_str: String,
    /// The oldest version it applies on top of.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_applicable_version: Option<u32>,
    /// The newest version it applies on top of.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_applicable_version: Option<u32>,
    /// The file's size in bytes.
    pub size: u64,
    #[serde(skip)]
    digest: Vec<u8>,
}

impl OtaImage {
    /// Whether a node of `vendor_id` and `product_id` running `version` can
    /// update to this image.
    pub fn is_update_for(&self, vendor_id: u16, product_id: u16, version: u32) -> bool {
        (self.vendor_id == 0 || self.vendor_id == vendor_id)
            && (self.product_id == 0 || self.product_id == product_id)
            && self.version > version
            && self.min_applicable_version.is_none_or(|v| version >= v)
            && self.max_applicable_version.is_none_or(|v| version <= v)
    }

    /// The update token of the offer: the version and the start of the
    /// image's digest, 8 to 32 bytes.
    fn update_token(&self) -> Vec<u8> {
        let mut token = self.version.to_be_bytes().to_vec();
        token.extend(self.digest.iter().take(12));
        token.resize(token.len().max(8), 0);
        token
    }
}

/// Whether `name` may be served: a plain file name, so a designator never
/// reaches outside the directory.
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// Read the header of the OTA image file at `path`; `None` if it is not one,
/// or its size disagrees with its header.
fn read_image(path: &Path) -> Option<OtaImage> {
    let file = path.file_name()?.to_str()?.to_string();
    if !valid_name(&file) {
        return None;
    }
    let f = std::fs::File::open(path).ok()?;
    let size = f.metadata().ok()?.len();
    let mut head = Vec::new();
    f.take(MAX_HEADER).read_to_end(&mut head).ok()?;
    let (header, payload_offset) = OtaImageHeader::decode(&head).ok()?;
    if (payload_offset as u64).checked_add(header.payload_size) != Some(size) {
        return None;
    }
    Some(OtaImage {
        file,
        vendor_id: header.vendor_id,
        product_id: header.product_id,
        version: header.software_version,
        version_str: header.software_version_str.to_string(),
        min_applicable_version: header.min_applicable_software_version,
        max_applicable_version: header.max_applicable_software_version,
        size,
        digest: header.image_digest.0.to_vec(),
    })
}

/// The OTA image files in `dir`, by name; files that are not images are
/// left out.
pub fn list(dir: &Path) -> Result<Vec<OtaImage>, crate::Error> {
    let mut images: Vec<OtaImage> = match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
            .filter_map(|e| read_image(&e.path()))
            .collect(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e.into()),
    };
    images.sort_by(|a, b| a.file.cmp(&b.file));
    Ok(images)
}

/// The newest image in `images` for a node of `vendor_id` and `product_id`
/// running `version`.
pub fn best(
    images: &[OtaImage],
    vendor_id: u16,
    product_id: u16,
    version: u32,
) -> Option<&OtaImage> {
    images
        .iter()
        .filter(|i| i.is_update_for(vendor_id, product_id, version))
        .max_by_key(|i| i.version)
}

/// The directory of images, as rs-matter's OTA provider sees it.
pub(crate) struct OtaDir {
    dir: PathBuf,
    events: EventBus,
}

impl OtaDir {
    pub(crate) fn new(dir: PathBuf, events: EventBus) -> Self {
        Self { dir, events }
    }

    fn path(&self, designator: &[u8]) -> Option<PathBuf> {
        let name = std::str::from_utf8(designator).ok()?;
        valid_name(name).then(|| self.dir.join(name))
    }
}

/// Copy `bytes` to the start of `buf`, returning the copy and the rest.
fn lend<'b>(buf: &'b mut [u8], bytes: &[u8]) -> Option<(&'b [u8], &'b mut [u8])> {
    let (head, rest) = buf.split_at_mut_checked(bytes.len())?;
    head.copy_from_slice(bytes);
    Some((head, rest))
}

impl OtaImagesRegistry for OtaDir {
    async fn query<'b>(
        &self,
        requestor: OtaRequestorId,
        vendor_id: u16,
        product_id: u16,
        current_version: u32,
        _requestor_can_consent: bool,
        buf: &'b mut [u8],
    ) -> OtaQueryOutcome<'b> {
        let images = match list(&self.dir) {
            Ok(images) => images,
            Err(e) => {
                log::warn!("OTA: cannot list {}: {e}", self.dir.display());
                return OtaQueryOutcome::NotAvailable;
            }
        };
        let Some(image) = best(&images, vendor_id, product_id, current_version) else {
            log::debug!(
                "OTA: nothing newer than {current_version} for node {:016X} ({vendor_id:#06x}/{product_id:#06x})",
                requestor.node_id
            );
            return OtaQueryOutcome::NotAvailable;
        };

        let token = image.update_token();
        let lent = lend(buf, image.version_str.as_bytes()).and_then(|(version_str, buf)| {
            let (file, buf) = lend(buf, image.file.as_bytes())?;
            let (token, _) = lend(buf, &token)?;
            Some((version_str, file, token))
        });
        // The strings were checked as ASCII on the way in.
        let Some((Ok(version_str), Ok(file_designator), update_token)) =
            lent.map(|(v, f, t)| (std::str::from_utf8(v), std::str::from_utf8(f), t))
        else {
            log::warn!("OTA: {} does not fit an offer", image.file);
            return OtaQueryOutcome::NotAvailable;
        };

        log::info!(
            "OTA: offering {} ({}) to node {:016X}",
            image.file,
            image.version_str,
            requestor.node_id
        );
        self.events.send(Event::Ota {
            node: requestor.node_id,
            step: OtaStep::Offered,
            version: image.version,
        });
        OtaQueryOutcome::Available(OtaImageMeta {
            version: image.version,
            version_str,
            file_designator,
            update_token,
            user_consent_needed: false,
        })
    }

    async fn apply(
        &self,
        requestor: OtaRequestorId,
        _update_token: &[u8],
        new_version: u32,
    ) -> OtaApplyOutcome {
        self.events.send(Event::Ota {
            node: requestor.node_id,
            step: OtaStep::Applying,
            version: new_version,
        });
        OtaApplyOutcome::Proceed { delay_secs: 0 }
    }

    async fn applied(
        &self,
        requestor: OtaRequestorId,
        _update_token: &[u8],
        software_version: u32,
    ) {
        self.events.send(Event::Ota {
            node: requestor.node_id,
            step: OtaStep::Applied,
            version: software_version,
        });
    }
}

impl OtaImages for OtaDir {
    async fn size(&self, designator: &[u8]) -> Option<u64> {
        let path = self.path(designator)?;
        read_image(&path).map(|i| i.size)
    }

    async fn read(
        &self,
        designator: &[u8],
        offset: u64,
        buf: &mut [u8],
    ) -> Result<usize, MatterError> {
        let path = self.path(designator).ok_or(ErrorCode::NotFound)?;
        let mut f = std::fs::File::open(path).map_err(|_| ErrorCode::NotFound)?;
        f.seek(SeekFrom::Start(offset))
            .map_err(|_| ErrorCode::Invalid)?;
        Ok(f.read(buf).map_err(|_| ErrorCode::Invalid)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rs_matter::ota::OTA_IMAGE_DIGEST_SHA256;
    use rs_matter::tlv::OctetStr;

    /// An OTA image file with `payload` for `vid`/`pid` at `version`.
    pub(crate) fn image(vid: u16, pid: u16, version: u32, payload: &[u8]) -> Vec<u8> {
        let version_str = format!("v{version}");
        let digest = [0xA5u8; 32];
        let header = OtaImageHeader {
            vendor_id: vid,
            product_id: pid,
            software_version: version,
            software_version_str: &version_str,
            payload_size: payload.len() as u64,
            min_applicable_software_version: None,
            max_applicable_software_version: None,
            release_notes_url: None,
            image_digest_type: OTA_IMAGE_DIGEST_SHA256,
            image_digest: OctetStr::new(&digest),
        };
        let mut buf = vec![0; 512];
        let n = header.encode(&mut buf).unwrap();
        buf.truncate(n);
        buf.extend_from_slice(payload);
        buf
    }

    #[test]
    fn picks_the_newest_applicable_image() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a-2.ota"), image(0xFFF1, 0x8001, 2, b"two")).unwrap();
        std::fs::write(
            dir.path().join("a-3.ota"),
            image(0xFFF1, 0x8001, 3, b"three"),
        )
        .unwrap();
        std::fs::write(
            dir.path().join("b-9.ota"),
            image(0xFFF1, 0x8002, 9, b"other"),
        )
        .unwrap();
        std::fs::write(dir.path().join("notes.txt"), b"not an image").unwrap();
        let mut truncated = image(0xFFF1, 0x8001, 4, b"four");
        truncated.pop();
        std::fs::write(dir.path().join("a-4.ota"), truncated).unwrap();

        let images = list(dir.path()).unwrap();
        assert_eq!(
            images.iter().map(|i| i.file.as_str()).collect::<Vec<_>>(),
            ["a-2.ota", "a-3.ota", "b-9.ota"]
        );
        assert_eq!(best(&images, 0xFFF1, 0x8001, 1).unwrap().file, "a-3.ota");
        assert_eq!(best(&images, 0xFFF1, 0x8001, 3), None);
        assert_eq!(best(&images, 0xFFF2, 0x8001, 1), None);
        let token = images[1].update_token();
        assert!((8..=32).contains(&token.len()));
    }

    #[test]
    fn designators_stay_in_the_directory() {
        for bad in [
            "",
            "../x.ota",
            "a/b.ota",
            ".hidden",
            "a b.ota",
            &"x".repeat(MAX_NAME + 1),
        ] {
            assert!(!valid_name(bad), "{bad:?}");
        }
        assert!(valid_name("light-1.2.3_rc1.ota"));
    }
}
