use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CwebpStatus {
    pub available: bool,
    #[serde(rename = "binaryPath")]
    pub binary_path: Option<String>,
    pub version: Option<String>,
    pub platform: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncodingConfig {
    pub quality: u8,
    #[serde(rename = "alphaQuality")]
    pub alpha_quality: u8,
    pub method: u8,
    pub lossless: bool,
    #[serde(rename = "nearLossless")]
    pub near_lossless: Option<u8>,
    pub metadata: String,
}

fn get_platform() -> String {
    if cfg!(target_os = "windows") {
        "windows".to_string()
    } else if cfg!(target_os = "macos") {
        "macos".to_string()
    } else {
        "linux".to_string()
    }
}

/// Detect cwebp binary using `which` crate with OS-specific fallback paths
pub async fn detect_cwebp() -> CwebpStatus {
    if let Ok(path) = which::which("cwebp") {
        let version = get_cwebp_version(&path).await;
        return CwebpStatus {
            available: true,
            binary_path: Some(path.to_string_lossy().to_string()),
            version,
            platform: get_platform(),
        };
    }

    let fallback_paths = get_fallback_paths();
    for path in fallback_paths {
        if path.exists() {
            let version = get_cwebp_version(&path).await;
            return CwebpStatus {
                available: true,
                binary_path: Some(path.to_string_lossy().to_string()),
                version,
                platform: get_platform(),
            };
        }
    }

    CwebpStatus {
        available: false,
        binary_path: None,
        version: None,
        platform: get_platform(),
    }
}

fn get_fallback_paths() -> Vec<PathBuf> {
    if cfg!(target_os = "windows") {
        let mut paths = vec![
            PathBuf::from(r"C:\Program Files\WebP\bin\cwebp.exe"),
            PathBuf::from(r"C:\Program Files (x86)\WebP\bin\cwebp.exe"),
        ];
        // Check user-local install paths
        if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
            let local_dir = PathBuf::from(&local_app_data).join("WebP");
            if let Ok(entries) = std::fs::read_dir(&local_dir) {
                for entry in entries.flatten() {
                    let candidate = entry.path().join("bin").join("cwebp.exe");
                    if candidate.exists() {
                        paths.insert(0, candidate);
                    }
                }
            }
        }
        paths
    } else if cfg!(target_os = "macos") {
        vec![
            PathBuf::from("/usr/local/bin/cwebp"),
            PathBuf::from("/opt/homebrew/bin/cwebp"),
        ]
    } else {
        vec![
            PathBuf::from("/usr/bin/cwebp"),
            PathBuf::from("/usr/local/bin/cwebp"),
            PathBuf::from("/snap/bin/cwebp"),
        ]
    }
}

async fn get_cwebp_version(binary_path: &PathBuf) -> Option<String> {
    let output = Command::new(binary_path)
        .arg("-version")
        .output()
        .await
        .ok()?;

    if output.status.success() {
        let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Some(version)
    } else {
        None
    }
}

/// Build cwebp command-line arguments from EncodingConfig
pub fn build_args(config: &EncodingConfig, input_path: &str, output_path: &str) -> Vec<String> {
    let mut args = Vec::new();

    if config.lossless {
        args.push("-lossless".to_string());

        if let Some(nl) = config.near_lossless {
            args.push("-near_lossless".to_string());
            args.push(nl.to_string());
        }
    } else {
        args.push("-q".to_string());
        args.push(config.quality.to_string());
    }

    args.push("-alpha_q".to_string());
    args.push(config.alpha_quality.to_string());

    args.push("-m".to_string());
    args.push(config.method.to_string());

    args.push("-metadata".to_string());
    args.push(config.metadata.clone());

    args.push("-o".to_string());
    args.push(output_path.to_string());

    args.push(input_path.to_string());

    args
}

/// Execute cwebp process asynchronously
pub async fn execute_cwebp(
    binary_path: &str,
    config: &EncodingConfig,
    input_path: &str,
    output_path: &str,
) -> Result<(), String> {
    let args = build_args(config, input_path, output_path);

    let output = Command::new(binary_path)
        .args(&args)
        .output()
        .await
        .map_err(|e| format!("Failed to execute cwebp: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Encoding failed: {}", stderr.trim()));
    }

    Ok(())
}
