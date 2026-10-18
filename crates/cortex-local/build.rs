use std::{env, error::Error, fs, path::Path, process::Command};

fn collect(directory: &Path, files: &mut Vec<std::path::PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            collect(&entry.path(), files)?;
        } else if kind.is_file() {
            files.push(entry.path());
        } else {
            return Err(std::io::Error::other(
                "Dashboard assets must be regular files",
            ));
        }
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let package = std::path::PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let workspace = package
        .parent()
        .and_then(Path::parent)
        .ok_or("Cortex package must live under crates/")?;
    let gui = workspace.join("gui");
    for relative in [
        "src",
        "public",
        "index.html",
        "vite.config.ts",
        "tsconfig.json",
        "tsconfig.electron.json",
        "package.json",
        "pnpm-lock.yaml",
        "scripts/build-standalone.mjs",
    ] {
        println!("cargo:rerun-if-changed={}", gui.join(relative).display());
    }
    for name in ["NODE_ENV", "NODE_OPTIONS"] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    let out = std::path::PathBuf::from(env::var("OUT_DIR")?);
    let dashboard = out.join("dashboard");
    let status = Command::new("node")
        .arg(gui.join("scripts/build-standalone.mjs"))
        .arg(&dashboard)
        .current_dir(&gui)
        .status()
        .map_err(|error| format!("Building the existing dashboard requires Node.js 22.12+ and `pnpm --dir gui install --frozen-lockfile`: {error}"))?;
    if !status.success() {
        return Err("Dashboard build failed. Install GUI dependencies with `pnpm --dir gui install --frozen-lockfile`; see renderer diagnostics.".into());
    }
    let mut files = Vec::new();
    collect(&dashboard, &mut files)?;
    files.sort();
    if files.is_empty() || !dashboard.join("index.html").is_file() {
        return Err("Dashboard build did not produce index.html".into());
    }
    let mut manifest = String::from("pub const ASSETS: &[(&str, &str, &[u8])] = &[\n");
    for file in files {
        let relative = file
            .strip_prefix(&dashboard)?
            .to_string_lossy()
            .replace('\\', "/");
        if !relative
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'_' | b'-'))
        {
            return Err(format!("Dashboard asset path must use URL-safe ASCII: {relative}").into());
        }
        let mime = match file.extension().and_then(|extension| extension.to_str()) {
            Some("html") => "text/html; charset=utf-8",
            Some("js") => "text/javascript; charset=utf-8",
            Some("css") => "text/css; charset=utf-8",
            Some("svg") => "image/svg+xml",
            Some("png") => "image/png",
            Some("woff2") => "font/woff2",
            Some("woff") => "font/woff",
            _ => return Err(format!("Unexpected dashboard asset: {relative}").into()),
        };
        let route = if relative == "index.html" {
            "/".into()
        } else {
            format!("/{relative}")
        };
        manifest.push_str(&format!(
            "({route:?}, {mime:?}, include_bytes!({:?})),\n",
            file.to_string_lossy()
        ));
    }
    manifest.push_str("];\n");
    fs::write(out.join("dashboard_assets.rs"), manifest)?;
    Ok(())
}
