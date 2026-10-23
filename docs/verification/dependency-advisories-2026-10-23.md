# Dependency advisory check — 23 October 2026

`npm audit --audit-level=high` previously reported zero vulnerabilities for this worktree's unchanged dependency graph. `cargo-audit 0.22.2` was installed into a task-specific cache and run against the locked Rust graph (743 dependencies). The RustSec database commit was `640bf5a9f06e66d8721e8772766531fe77c8630d` with 1,349 advisories. The command exited successfully under its default policy, with **15 allowed warnings**. This is not a clean advisory result.

Fourteen warnings concern unmaintained packages: GTK3 layer-shell bindings, `proc-macro-error`, the `unic-*` family and several `zbus_names`/`zvariant`/derive versions. These remain in the desktop dependency graph. No advisory suppression or blanket dependency update was added.

The remaining warning is [RUSTSEC-2024-0429](https://rustsec.org/advisories/RUSTSEC-2024-0429): `glib 0.18.5` has unsound `VariantStrIter` iteration. Inspection confirmed that its local `impl_get` passes an immutable pointer out-argument to the C function. RustSec lists `>=0.20.0` as patched. Reflow's Linux Tauri/GTK3 stack resolves GLib 0.18 through GTK, WebKit, tray/menu and layer-shell dependencies; simply adding a newer direct GLib dependency does not replace that ABI's transitive copy. No direct `array_iter_str`/`VariantStrIter` calls were found in Reflow source. This observation is not proof of unreachability throughout native libraries or future changes.

The [GTK3 layer-shell maintenance advisory](https://rustsec.org/advisories/RUSTSEC-2024-0422) has no patched version in that crate line. Replacing the desktop GTK stack or adopting a reviewed compatible backport needs separate compatibility validation. Do not describe the current Rust graph as free of known advisory findings, and do not treat a successful default `cargo audit` exit as resolving these warnings.

Raw local scan: `/tmp/reflow-cargo-audit.log`. Dependency trace: `/tmp/reflow-glib-dependents.log`. These scans establish reported dependency findings at this database revision; they are not a whole-application security certification.
