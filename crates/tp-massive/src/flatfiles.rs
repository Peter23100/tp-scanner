//! One trading day of full-market minute bars from Massive's flat files (S3), for replay.
//!
//! Files land by 11:00 ET the next day. Path: `us_stocks_sip/minute_aggs_v1/YYYY/MM/YYYY-MM-DD.csv.gz`
//! in bucket `flatfiles` at `https://files.massive.com` (⚠ the Polygon-era layout; Massive's page
//! confirms the `us_stocks_sip` prefix and the endpoint, and a 404 here means the layout moved).
//! The S3 keys are separate from the API key: Dashboard → Flat Files.

use crate::{MassiveError, Secret};
use chrono::{Datelike, NaiveDate};
use rusty_s3::{Bucket, Credentials, S3Action, UrlStyle};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const ENDPOINT: &str = "https://files.massive.com";
pub const BUCKET: &str = "flatfiles";

pub fn minute_aggs_key(d: NaiveDate) -> String {
    format!("us_stocks_sip/minute_aggs_v1/{:04}/{:02}/{d}.csv.gz", d.year(), d.month())
}

/// Where a downloaded day is kept: `<data>/flatfiles/minute_aggs/YYYY-MM-DD.csv.gz`.
pub fn local_path(data_dir: &Path, d: NaiveDate) -> PathBuf {
    data_dir.join("flatfiles").join("minute_aggs").join(format!("{d}.csv.gz"))
}

/// Download `d` unless it is already on disk. Returns the local path.
pub fn fetch_minute_aggs(key_id: &Secret, secret: &Secret, data_dir: &Path, d: NaiveDate) -> Result<PathBuf, MassiveError> {
    let dest = local_path(data_dir, d);
    if dest.exists() {
        return Ok(dest);
    }
    let bucket = Bucket::new(url::Url::parse(ENDPOINT).expect("endpoint"), UrlStyle::Path, BUCKET, "us-east-1")
        .map_err(|e| MassiveError::Parse(e.to_string()))?;
    let creds = Credentials::new(key_id.expose(), secret.expose());
    let object = minute_aggs_key(d);
    let signed = bucket.get_object(Some(&creds), &object).sign(Duration::from_secs(600));
    let http = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(600))
        .build()
        .map_err(|e| MassiveError::Http(e.to_string()))?;
    let mut r = http.get(signed).send().map_err(|e| MassiveError::Http(e.without_url().to_string()))?;
    if !r.status().is_success() {
        let status = r.status().as_u16();
        let body: String = r.text().unwrap_or_default().chars().take(300).collect();
        return Err(MassiveError::Status { status, url: format!("{ENDPOINT}/{BUCKET}/{object}"), body });
    }
    let dir = dest.parent().expect("has parent");
    std::fs::create_dir_all(dir).map_err(|e| MassiveError::Io(format!("{}: {e}", dir.display())))?;
    let tmp = dest.with_extension("gz.tmp");
    let mut f = std::fs::File::create(&tmp).map_err(|e| MassiveError::Io(format!("{}: {e}", tmp.display())))?;
    r.copy_to(&mut f).map_err(|e| MassiveError::Http(e.without_url().to_string()))?;
    f.flush().map_err(|e| MassiveError::Io(e.to_string()))?;
    drop(f);
    std::fs::rename(&tmp, &dest).map_err(|e| MassiveError::Io(format!("{}: {e}", dest.display())))?;
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_keys_follow_the_dated_layout() {
        let d = NaiveDate::from_ymd_opt(2026, 10, 9).unwrap();
        assert_eq!(minute_aggs_key(d), "us_stocks_sip/minute_aggs_v1/2026/10/2026-10-09.csv.gz");
        assert!(local_path(Path::new("C:/x"), d).ends_with("flatfiles/minute_aggs/2026-10-09.csv.gz"));
    }
}
