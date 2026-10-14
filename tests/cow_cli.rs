use cpt32::cow::{BaseImage, CowOverlay};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

struct TempDir(PathBuf);
impl TempDir {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("cpt32-cow-cli-{}-{nonce}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn verify_bake_preserves_inputs_and_refuses_existing_output() {
    let temp = TempDir::new();
    let base = temp.path("base.img");
    let cow = temp.path("disk.cow");
    let output = temp.path("baked.img");
    let original: Vec<u8> = (0..1024).map(|i| (i % 251) as u8).collect();
    let changed = vec![0xA5; 512];
    fs::write(&base, &original).unwrap();
    let mut overlay =
        CowOverlay::open(BaseImage::Bytes(Arc::from(original.clone())), &cow, 1, 512).unwrap();
    overlay.write_block(1, &changed).unwrap();
    drop(overlay);
    OpenOptions::new()
        .append(true)
        .open(&cow)
        .unwrap()
        .write_all(b"BLK")
        .unwrap();
    let cow_before = fs::read(&cow).unwrap();

    let verified = Command::new(env!("CARGO_BIN_EXE_cpt32-tool"))
        .args([
            "cow",
            "verify",
            base.to_str().unwrap(),
            cow.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        verified.status.success(),
        "{}",
        String::from_utf8_lossy(&verified.stderr)
    );
    let verified_stdout = String::from_utf8_lossy(&verified.stdout);
    assert!(verified_stdout.contains("records: 1"));
    assert!(verified_stdout.contains("unique blocks: 1"));
    assert!(verified_stdout.contains("recoverable tail: true"));
    let baked = Command::new(env!("CARGO_BIN_EXE_cpt32-tool"))
        .args([
            "cow",
            "bake",
            base.to_str().unwrap(),
            cow.to_str().unwrap(),
            "-o",
            output.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        baked.status.success(),
        "{}",
        String::from_utf8_lossy(&baked.stderr)
    );
    let baked_stdout = String::from_utf8_lossy(&baked.stdout);
    assert!(baked_stdout.contains("blocks applied: 1"));
    assert!(baked_stdout.contains("bytes written: 1024"));
    let mut expected = original.clone();
    expected[512..].copy_from_slice(&changed);
    assert_eq!(fs::read(&output).unwrap(), expected);
    assert_eq!(fs::read(&base).unwrap(), original);
    assert_eq!(fs::read(&cow).unwrap(), cow_before);

    let refused = Command::new(env!("CARGO_BIN_EXE_cpt32-tool"))
        .args([
            "cow",
            "bake",
            base.to_str().unwrap(),
            cow.to_str().unwrap(),
            "-o",
            output.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!refused.status.success());
    assert_eq!(fs::read(&output).unwrap(), expected);
}
