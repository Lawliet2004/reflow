// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use reflow_lib::history::HistoryStore;
use reflow_lib::platform::PlatformSys;
use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() > 1 {
        let cmd = args[1].as_str();
        match cmd {
            "--status" | "status" => {
                println!("{}", PlatformSys::generate_diagnostics_report());
                return;
            }
            "--history-list" | "history" => {
                let db_path = PlatformSys::get_db_path();
                if let Ok(store) = HistoryStore::new(db_path) {
                    if let Ok(entries) = store.get_entries(20, 0) {
                        println!("=== Reflow Dictation History (Latest 20) ===");
                        for e in entries {
                            println!(
                                "[{}] ({}s | {}) {}",
                                e.created_at,
                                e.duration_ms / 1000,
                                e.language,
                                e.final_transcript
                            );
                        }
                    }
                }
                return;
            }
            // Machine-readable form of exactly what the diagnostics view
            // shows, so two runs can be diffed by a script.
            "--latency-json" => {
                match reflow_lib::state::load_latency_report() {
                    Some(report) => println!(
                        "{}",
                        serde_json::to_string_pretty(&report).expect("latency report serializes")
                    ),
                    None => {
                        eprintln!(
                            "No latency measurements recorded yet. Complete a dictation first ({}).",
                            reflow_lib::state::latency_report_path().display()
                        );
                        std::process::exit(1);
                    }
                }
                return;
            }
            "--benchmark" if args.len() > 2 => {
                match reflow_lib::benchmark::runtime::run_runtime_benchmark(std::path::Path::new(
                    &args[2],
                )) {
                    Ok(report) => println!(
                        "{}",
                        serde_json::to_string_pretty(&report).expect("benchmark serializes")
                    ),
                    Err(error) => {
                        eprintln!("{error}");
                        std::process::exit(1);
                    }
                }
                return;
            }
            "--latency" | "--benchmark" | "benchmark" => {
                let metrics = PlatformSys::get_system_metrics();
                println!("=== Reflow latency report ===");
                println!("CPU usage: {:.1}%", metrics.cpu_usage_pct);
                println!("App RAM: {:.1} MB", metrics.app_ram_mb);
                println!("VRAM: {:.1} MB", metrics.vram_mb);
                println!("GPU: {}", metrics.gpu_name);
                println!();

                // Report measurements or say plainly that there are none. The
                // previous implementation printed fixed "simulated" numbers
                // and an unconditional PASSED, which made every latency claim
                // in the project unfalsifiable.
                match reflow_lib::state::load_latency_report() {
                    None => {
                        println!("No dictation latency recorded yet.");
                        println!("Run a dictation, then re-run this command.");
                    }
                    Some(report) => {
                        let p = &report.percentiles;
                        let last = &report.last;
                        println!("Samples in window: {}", p.samples);
                        println!(
                            "Last dictation: hotkey→rec {} ms | rec→audio {} ms | \
                             audio→partial {} ms | release→final {} ms",
                            last.hotkey_to_recording_ms,
                            last.recording_to_first_audio_ms,
                            last.audio_to_first_partial_ms,
                            last.speech_end_to_final_ms
                        );
                        println!(
                            "                llm start {} ms | format {} ms | rewrite {} ms \
                             (applied={})",
                            last.llm_startup_ms,
                            last.formatting_ms,
                            last.rewrite_ms,
                            last.rewrite_applied
                        );
                        println!(
                            "                release→inserted {} ms | total {} ms | \
                             audio {} ms | RTF {:.3}",
                            last.release_to_inserted_ms,
                            last.total_duration_ms,
                            last.audio_duration_ms,
                            last.rtf
                        );
                        println!(
                            "Rolling p50/p95: hotkey→rec {}/{} ms | release→final {}/{} ms",
                            p.hotkey_to_recording_p50_ms,
                            p.hotkey_to_recording_p95_ms,
                            p.speech_end_to_final_p50_ms,
                            p.speech_end_to_final_p95_ms
                        );
                        println!(
                            "                 rewrite {}/{} ms | release→inserted {}/{} ms",
                            p.rewrite_p50_ms,
                            p.rewrite_p95_ms,
                            p.release_to_inserted_p50_ms,
                            p.release_to_inserted_p95_ms
                        );
                        println!(
                            "                 RTF {:.3}/{:.3} | LLM applied {:.0}%",
                            p.rtf_p50,
                            p.rtf_p95,
                            p.llm_applied_rate * 100.0
                        );
                        println!();
                        println!(
                            "Acceptance thresholds are enforced by the benchmark harness \
                             (Milestone 7), not by this command."
                        );
                    }
                }
                return;
            }
            "--api" | "api" => {
                let bind = if args.get(2).map(|s| s.starts_with('-')).unwrap_or(true) {
                    args.iter()
                        .position(|a| a == "--bind")
                        .and_then(|i| args.get(i + 1).cloned())
                } else {
                    args.get(2).cloned()
                };
                let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
                if let Err(err) = rt.block_on(reflow_lib::run_api_standalone(bind)) {
                    eprintln!("LAN API failed: {err}");
                    std::process::exit(1);
                }
                return;
            }
            "--pair-reset" => {
                let ctx = reflow_lib::context::AppContext::bootstrap();
                if let Err(err) = ctx.pairing.reset() {
                    eprintln!("{err}");
                    std::process::exit(1);
                }
                println!("Paired Android devices cleared.");
                return;
            }
            "--help" | "-h" | "help" => {
                println!("Reflow — Local Wispr Flow-Style Desktop Dictation Application");
                println!("\nUsage:");
                println!("  reflow [OPTIONS]");
                println!("\nOptions:");
                println!("  --status        Display current hardware and model diagnostics");
                println!("  --history-list  List latest local SQLite transcriptions");
                println!("  --latency       Print the measured latency waterfall and p50/p95");
                println!("  --latency-json  Same numbers as JSON, for scripted comparison");
                println!("  --api [--bind HOST:PORT]  Headless LAN API for Android");
                println!("  --pair-reset    Forget all paired Android devices");
                println!("  --help          Print help information");
                return;
            }
            _ => {}
        }
    }

    reflow_lib::run();
}
