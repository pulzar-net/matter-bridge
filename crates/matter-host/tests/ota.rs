//! A software update over Matter OTA: the controller commissions a device
//! with an OTA Requestor (CI runs connectedhomeip's `chip-ota-requestor-app`
//! in a container), announces itself as its provider, offers it the image in
//! its OTA directory and serves it over BDX.
//!
//! Ignored by default: it needs the device. Run with
//! `cargo test -p matter-host --test ota -- --ignored`, with
//!
//! - `MATTER_OTA_CODE`: the device's pairing code (default: the CSA test
//!   device's, `34970112332`);
//! - `MATTER_OTA_INTERFACE`: the interface to discover it on (optional);
//! - `MATTER_OTA_DOWNLOAD`: where the device writes the image it downloads
//!   (default `/tmp/ota/downloaded.ota`).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use matter_host::{CommissionRequest, Config, Controller, Event, Networks, OtaStep};
use rs_matter::ota::{OtaImageHeader, OTA_IMAGE_DIGEST_SHA256};
use rs_matter::tlv::OctetStr;

/// An OTA image for any vendor and product at `version`, with `payload`.
fn image(version: u32, payload: &[u8]) -> Vec<u8> {
    let version_str = format!("{version}.0");
    let digest = [0x5Au8; 32];
    let header = OtaImageHeader {
        vendor_id: 0,
        product_id: 0,
        software_version: version,
        software_version_str: &version_str,
        payload_size: payload.len() as u64,
        min_applicable_software_version: None,
        max_applicable_software_version: None,
        release_notes_url: None,
        image_digest_type: OTA_IMAGE_DIGEST_SHA256,
        image_digest: OctetStr::new(&digest),
    };
    let mut file = vec![0; 512];
    let n = header.encode(&mut file).expect("header");
    file.truncate(n);
    file.extend_from_slice(payload);
    file
}

#[tokio::test]
#[ignore = "needs a Matter device with an OTA Requestor"]
async fn update_over_ota() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,matter_host=debug".into()),
        )
        .with_test_writer()
        .try_init();
    let code = std::env::var("MATTER_OTA_CODE").unwrap_or_else(|_| "34970112332".into());
    let download = PathBuf::from(
        std::env::var("MATTER_OTA_DOWNLOAD").unwrap_or_else(|_| "/tmp/ota/downloaded.ota".into()),
    );

    // 64 KiB: many BDX blocks.
    let payload: Vec<u8> = (0..64 * 1024u32).map(|i| (i * 7 + i / 251) as u8).collect();
    let file = image(100, &payload);

    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::new(dir.path());
    config.interface = std::env::var("MATTER_OTA_INTERFACE").ok();
    std::fs::create_dir_all(&config.ota_dir).unwrap();
    std::fs::write(config.ota_dir.join("update-100.ota"), &file).unwrap();

    let controller = Controller::start(config).expect("controller");
    let images = controller.ota_images().unwrap();
    assert_eq!(images.len(), 1, "{images:?}");
    assert_eq!(images[0].version, 100);
    let mut events = controller.events();

    let node = controller
        .commission(CommissionRequest {
            code,
            address: None,
            name: Some("ota-requestor".into()),
            discovery_timeout: 60,
            networks: Networks::default(),
        })
        .await
        .expect("commission");
    log::info!("commissioned node {:016X}", node.node_id);

    let offered = controller
        .announce_ota(node.node_id)
        .await
        .expect("announce");
    assert_eq!(offered.map(|i| i.file).as_deref(), Some("update-100.ota"));

    // The device asks, is offered the image, downloads it over BDX and asks
    // to apply it; the download is checked byte for byte.
    let deadline = Instant::now() + Duration::from_secs(240);
    let mut steps = Vec::new();
    loop {
        assert!(
            Instant::now() < deadline,
            "no complete download; provider steps {steps:?}"
        );
        while let Ok(event) = events.try_recv() {
            if let Event::Ota {
                node: n,
                step,
                version,
            } = event
            {
                assert_eq!(n, node.node_id);
                assert_eq!(version, 100);
                log::info!("OTA step {step:?}");
                steps.push(step);
            }
        }
        let downloaded = std::fs::read(&download).unwrap_or_default();
        // The requestor keeps the payload, or the whole file.
        let complete = downloaded == payload || downloaded == file;
        if complete || steps.contains(&OtaStep::Applying) {
            assert!(steps.contains(&OtaStep::Offered), "{steps:?}");
            log::info!(
                "update downloaded ({} bytes on disk), provider steps {steps:?}",
                downloaded.len()
            );
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}
