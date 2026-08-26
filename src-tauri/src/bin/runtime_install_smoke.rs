// Standalone smoke test for the runtime installer. Invoked by
// `cargo run --bin runtime_install_smoke`. This exists so that the
// runtime install behavior can be verified on machines where the
// `cargo test` test binary won't load (Windows DLL loader issues).
//
// We don't try to download anything here; we just exercise the
// pure-function code paths: spec selection, archive-kind detection,
// SHA-256 verification, and the env-var override.

use reflow_lib::rewrite::runtime_install::{
    pick_runtime_spec, ArchiveKind, RUNTIME_LOCK_KEY,
};
use sha2::{Digest, Sha256};
use std::io::Write;

fn main() {
    let mut failures = 0u32;
    run_basic_tests(&mut failures);

    // Optional: live round-trip — download the real CPU asset, verify
    // its SHA-256, extract the binary. Skipped by default because
    // it hits the network and downloads 18 MB. Opt in with
    // REFLOW_RUNTIME_LIVE_TEST=1.
    if std::env::var("REFLOW_RUNTIME_LIVE_TEST").ok().as_deref() == Some("1") {
        run_live_download(&mut failures);
    }

    if failures > 0 {
        eprintln!("\n{} smoke test(s) failed", failures);
        std::process::exit(1);
    }
    println!("\nAll smoke tests passed.");
}

fn run_basic_tests(failures: &mut u32) {
    // Archive kind detection.
    pass(failures, "ArchiveKind::from_extension detects .zip", ArchiveKind::from_extension("foo.zip") == ArchiveKind::Zip);
    pass(failures, "ArchiveKind::from_extension detects .tar.gz", ArchiveKind::from_extension("foo.tar.gz") == ArchiveKind::TarGz);

    // Lock key is stable.
    pass(failures, "RUNTIME_LOCK_KEY is 'llama-runtime'", RUNTIME_LOCK_KEY == "llama-runtime");

    // pick_runtime_spec returns a CPU spec when the user asks for it.
    let cpu = pick_runtime_spec("cpu");
    pass(failures, "pick_runtime_spec('cpu') returns Some", cpu.is_some());
    if let Some(spec) = cpu {
        pass(failures, "CPU spec is labelled 'CPU'", spec.kind_label == "CPU");
        pass(failures, "CPU spec has 64-char SHA-256", spec.sha256.len() == 64);
        pass(failures, "CPU spec URL points at the pinned tag", spec.url.contains("b10621"));
        pass(
            failures,
            "CPU spec binary_basename matches the platform",
            spec.binary_basename == if cfg!(windows) { "llama-server.exe" } else { "llama-server" },
        );
    }

    // pick_runtime_spec handles "auto" and unknown values.
    pass(failures, "pick_runtime_spec('auto') returns Some", pick_runtime_spec("auto").is_some());
    pass(failures, "pick_runtime_spec('garbage') returns Some", pick_runtime_spec("garbage").is_some());

    // SHA-256 of a known string.
    let mut hasher = Sha256::new();
    hasher.update(b"hello world");
    let digest = hasher.finalize();
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    pass(
        failures,
        "sha2 round-trip on 'hello world'",
        hex == "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9",
    );

    // SHA-256 of a small file written to disk.
    let tmp = std::env::temp_dir().join("reflow_sha_smoke.bin");
    {
        let mut f = std::fs::File::create(&tmp).expect("create temp");
        f.write_all(b"abc").expect("write");
    }
    let bytes = std::fs::read(&tmp).expect("read temp");
    let mut h = Sha256::new();
    h.update(&bytes);
    let got: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    pass(
        failures,
        "sha256('abc') matches the canonical digest",
        got == "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
    );
    let _ = std::fs::remove_file(&tmp);
}

fn pass(failures: &mut u32, name: &str, ok: bool) {
    if ok {
        println!("ok  - {}", name);
    } else {
        println!("FAIL - {}", name);
        *failures += 1;
    }
}

fn run_live_download(failures: &mut u32) {
    use reflow_lib::rewrite::runtime_install::{llama_server_bin, ENV_OVERRIDE_BIN};
    use std::io::Read;

    println!("\nLive download test (REFLOW_RUNTIME_LIVE_TEST=1)");

    // Back up any existing runtime, then ensure ENV_OVERRIDE_BIN is unset
    // so the default download path is exercised.
    let prev_override = std::env::var(ENV_OVERRIDE_BIN).ok();
    unsafe {
        std::env::remove_var(ENV_OVERRIDE_BIN);
    }
    let dest = llama_server_bin();
    let backup = std::env::temp_dir().join("llama-server.smoke.bak");
    if dest.exists() {
        let _ = std::fs::rename(&dest, &backup);
    }

    let spec = pick_runtime_spec("cpu").expect("cpu spec");
    let _ = std::fs::create_dir_all(dest.parent().unwrap());

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .expect("client");
    let mut resp = client.get(&spec.url).send().expect("GET");
    let status = resp.status();
    let http_ok = status.is_success();
    pass(
        failures,
        &format!("HTTP 200 on the CPU asset URL (got {status})"),
        http_ok,
    );
    if !http_ok {
        if let Some(v) = prev_override {
            unsafe {
                std::env::set_var(ENV_OVERRIDE_BIN, v);
            }
        }
        return;
    }
    let tmp = std::env::temp_dir().join("reflow_runtime_smoke.bin");
    {
        let mut f = std::fs::File::create(&tmp).expect("create tmp");
        let mut h = Sha256::new();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = resp.read(&mut buf).expect("read");
            if n == 0 {
                break;
            }
            h.update(&buf[..n]);
            f.write_all(&buf[..n]).expect("write");
        }
        let got: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
        pass(
            failures,
            &format!("Live SHA-256 matches the pinned digest (got {got})"),
            got.eq_ignore_ascii_case(&spec.sha256),
        );
    }

    // Extract the binary.
    let archive = std::fs::File::open(&tmp).expect("open tmp");
    let mut zip = zip::ZipArchive::new(archive).expect("zip");
    let mut extracted = false;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).expect("entry");
        if entry.name().rsplit('/').next() == Some(spec.binary_basename.as_str()) {
            let mut out = std::fs::File::create(&dest).expect("create dest");
            std::io::copy(&mut entry, &mut out).expect("extract");
            extracted = true;
            break;
        }
    }
    pass(failures, "Archive contains the expected binary entry", extracted);
    let exists = dest.exists();
    pass(failures, "Extracted binary exists on disk", exists);
    let impl_path = dest.with_file_name("llama-server-impl.dll");
    let impl_exists = impl_path.exists();
    pass(
        failures,
        "Companion llama-server-impl.dll extracted alongside the launcher",
        impl_exists,
    );
    if exists {
        let mut f = std::fs::File::open(&dest).expect("open dest");
        let mut magic = vec![0u8; 4];
        let n = f.read(&mut magic).unwrap_or(0);
        pass(
            failures,
            &format!(
                "Extracted binary starts with MZ (PE) — got {:?} ({} bytes)",
                &magic[..n],
                n
            ),
            n >= 2 && &magic[..2] == b"MZ",
        );
        let _ = std::fs::remove_file(&dest);
    }
    if impl_exists {
        let _ = std::fs::remove_file(&impl_path);
    }

    let _ = std::fs::remove_file(&tmp);
    if backup.exists() {
        let _ = std::fs::rename(&backup, &dest);
    }
    if let Some(v) = prev_override {
        unsafe {
            std::env::set_var(ENV_OVERRIDE_BIN, v);
        }
    }
}
