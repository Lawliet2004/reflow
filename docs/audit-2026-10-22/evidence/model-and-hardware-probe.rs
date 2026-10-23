use reflow_lib::{model::ModelManager,capability::{Capabilities,GpuInfo,GpuVendor},profile::{select_asr_load, Preset,no_measurements}};
fn main(){
 let root=std::env::temp_dir().join(format!("reflow-audit-{}",std::process::id()));
 let manager=ModelManager::new(root.clone());
 let model=manager.get_model_dir("0.6b");std::fs::create_dir_all(&model).unwrap();
 std::fs::write(model.join("reflow-install.json"),b"{}").unwrap();
 println!("Qwen receipt alone installed={} weights_exist={}",manager.is_installed("0.6b"),model.join("model.safetensors").exists());
 let z=manager.get_model_dir("zipformer-20m");std::fs::create_dir_all(&z).unwrap();
 let manifest=reflow_lib::profile::asr_manifest("zipformer-20m").unwrap();
 for f in std::iter::once(manifest.filename).chain(manifest.auxiliary_files.iter().map(|f|f.filename)){std::fs::write(z.join(f),b"").unwrap();}
 println!("Zipformer zero-byte files installed={}",manager.is_installed("zipformer-20m"));
 let mut caps=Capabilities::default();caps.ram.available_mb=256.;
 let s=select_asr_load(Preset::Auto,"1.7b","auto","auto",&|_|false,&caps,&no_measurements);
 println!("CPU 256MiB free selection={} device={:?} error={:?}",s.model_id,s.device,s.error);
 caps.gpus.push(GpuInfo{index:0,name:"NVIDIA fixture".into(),vendor:GpuVendor::Nvidia,total_vram_mb:12000.,free_vram_mb:11000.,used_vram_mb:1000.,driver_version:Some("fixture".into()),compute_capability:Some("8.6".into())});caps.cuda.nvidia_gpu_present=true;caps.cuda.driver_present=true;caps.cuda.driver_cuda_version=Some("13.0".into());
 let s=select_asr_load(Preset::Auto,"0.6b","auto","auto",&|_|false,&caps,&no_measurements);
 println!("GPU driver present, unprobed speech runtime: device={:?}",s.device);
 std::fs::remove_dir_all(root).unwrap();
}
