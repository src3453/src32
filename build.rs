use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=Cargo.lock");
    // Resolve paths through Git so linked worktrees and packed refs work too.
    let mut refs = vec!["HEAD".to_owned(), "packed-refs".to_owned()];
    if let Some(head_ref) = git(&["symbolic-ref", "-q", "HEAD"]) {
        refs.push(head_ref);
    }
    for reference in refs {
        if let Some(path) = git(&["rev-parse", "--git-path", &reference]) {
            let mut path = PathBuf::from(path);
            // Missing loose/packed refs are normal. Watching a missing file
            // would make every unchanged Cargo invocation rebuild the crate.
            while !path.exists() && path.pop() {}
            if path.exists() {
                println!("cargo:rerun-if-changed={}", path.display());
            }
        }
    }
    let count = git(&["rev-list", "--count", "HEAD"])
        .filter(|value| !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()))
        .unwrap_or_else(|| {
            println!("cargo:warning=Git commit count unavailable; build number is unknown");
            "unknown".to_owned()
        });
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("build clock before Unix epoch")
        .as_secs();
    println!("cargo:rustc-env=CPT32_BUILD_NUMBER={count}");
    println!(
        "cargo:rustc-env=CPT32_BUILD_TIME={}",
        utc_timestamp(seconds)
    );
}

fn utc_timestamp(seconds: u64) -> String {
    let mut days = seconds / 86_400;
    let mut year = 1970;
    let leap = |year| year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    loop {
        let length = if leap(year) { 366 } else { 365 };
        if days < length {
            break;
        }
        days -= length;
        year += 1;
    }
    let months = [
        31,
        if leap(year) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 1;
    for length in months {
        if days < length {
            break;
        }
        days -= length;
        month += 1;
    }
    let time = seconds % 86_400;
    format!(
        "{year:04}/{month:02}/{:02} {:02}:{:02}:{:02} UTC",
        days + 1,
        time / 3600,
        time / 60 % 60,
        time % 60
    )
}

#[cfg(test)]
mod tests {
    use super::utc_timestamp;

    #[test]
    fn utc_calendar_boundaries() {
        assert_eq!(utc_timestamp(0), "1970/01/01 00:00:00 UTC");
        assert_eq!(utc_timestamp(951_782_399), "2000/02/28 23:59:59 UTC");
        assert_eq!(utc_timestamp(951_782_400), "2000/02/29 00:00:00 UTC");
        assert_eq!(utc_timestamp(4_107_542_400), "2100/03/01 00:00:00 UTC");
    }
}
