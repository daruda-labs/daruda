//! Where a markdown image's bytes come from. Decoding is
//! `daruda_content::visual`'s; resolving a reference against the file system
//! is the workspace's, through the same resolver resource links use.

use std::path::Path;

use daruda_content::link_target;

/// Resolve a markdown image reference to its encoded bytes.
///
/// Policy (locked): only local files and `data:` URIs are loaded — a local
/// path resolves as a tool's resource link does (`file://` for this machine,
/// absolute, or relative to `base_dir`). Any other URL is refused: a local
/// file viewer must not fetch from the network.
pub(super) fn load_image_source(url: &str, base_dir: &Path) -> anyhow::Result<Vec<u8>> {
    if let Some(rest) = url.strip_prefix("data:") {
        let (meta, payload) = rest
            .split_once(',')
            .ok_or_else(|| anyhow::anyhow!("malformed data URI"))?;
        if meta.contains("base64") {
            use base64::Engine;
            Ok(base64::engine::general_purpose::STANDARD.decode(payload)?)
        } else {
            Ok(payload.as_bytes().to_vec())
        }
    } else {
        let Some(path) = link_target::resource_path(url, Some(base_dir)) else {
            anyhow::bail!("remote images are not fetched: {url}")
        };
        Ok(std::fs::read(path)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_png(width: u32, height: u32, px: [u8; 4]) -> Vec<u8> {
        let mut img = image::RgbaImage::new(width, height);
        for p in img.pixels_mut() {
            *p = image::Rgba(px);
        }
        let mut buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Png)
            .expect("encode png");
        buf.into_inner()
    }

    #[test]
    fn refuses_remote_image_urls() {
        let dir = std::env::temp_dir();
        assert!(load_image_source("https://example.com/a.png", &dir).is_err());
        assert!(load_image_source("http://example.com/a.png", &dir).is_err());
    }

    /// Refused even when joining it onto the base dir would name a real file.
    /// Unix only: `:` cannot appear in a Windows file name.
    #[cfg(unix)]
    #[test]
    fn refuses_other_url_schemes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let decoy = dir.path().join("mcp:/server");
        std::fs::create_dir_all(&decoy).expect("mkdir");
        std::fs::write(decoy.join("a.png"), b"x").expect("write decoy");
        assert!(load_image_source("mcp://server/a.png", dir.path()).is_err());
    }

    /// A `file://` reference is the same file a bare path names — the one
    /// resolver the agent chat's resource links use.
    #[test]
    fn reads_local_image_from_a_file_url() {
        let dir = tempfile::tempdir().expect("tempdir");
        let png = encode_png(2, 2, [7, 8, 9, 255]);
        let path = dir.path().join("with space.png");
        std::fs::write(&path, &png).expect("write png");
        let url = daruda_core::file_url::from_local_path(&path).expect("file url");
        assert_eq!(
            load_image_source(&url, Path::new("/unrelated")).expect("load"),
            png
        );
    }

    #[test]
    fn reads_local_image_relative_to_base_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let png = encode_png(2, 2, [1, 2, 3, 255]);
        std::fs::write(dir.path().join("pic.png"), &png).expect("write png");
        let bytes = load_image_source("pic.png", dir.path()).expect("load local");
        assert_eq!(bytes, png);
    }

    #[test]
    fn decodes_base64_data_uri() {
        use base64::Engine;
        let png = encode_png(2, 2, [4, 5, 6, 255]);
        let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
        let uri = format!("data:image/png;base64,{b64}");
        let bytes = load_image_source(&uri, &std::env::temp_dir()).expect("load data uri");
        assert_eq!(bytes, png);
    }
}
