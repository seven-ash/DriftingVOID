// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Ash (seven-ash). Additional terms (GPL-3.0 section 7): see NOTICE.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod anim;
mod app;
mod audio;
mod calc;
mod dial;
mod edges;
mod canvas;
mod doc;
mod export;
mod extrude;
mod fontdb;
mod glass;
mod glass_gpu;
mod i18n;
mod gpu;
mod gpu_paths;
#[cfg(test)]
mod golden;
mod okhsv;
mod penhook;
mod persp;
mod playcache;
mod theme;
mod io;
mod kit;
mod lines;
mod lsd;
mod oledrop;
mod raster;
mod recovery;
mod scene;
mod segment;
mod selftest;
mod text;
mod update;
mod vcache;
mod vector;
mod video;
mod vexport;
mod view3d;

fn main() -> eframe::Result {
    install_crash_log();
    let args: Vec<String> = std::env::args().skip(1).collect();
    fontdb::start_scan();
    if args.first().map(String::as_str) == Some("--selftest") {
        // wait for the font scan so SB_FONT can resolve
        while fontdb::families().is_none() {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        if args.get(1).map(String::as_str) == Some("--list-fonts") {
            let mut s = String::new();
            for f in fontdb::families().unwrap().iter() {
                s += &format!("{} :: {}
", f.name, f.faces.iter().map(|x| x.style.as_str()).collect::<Vec<_>>().join(", "));
            }
            let _ = std::fs::write(args.get(2).cloned().unwrap_or_else(|| "fonts.txt".into()), s);
            return Ok(());
        }
        let out = args.get(1).cloned().unwrap_or_else(|| "selftest.png".into());
        if let Err(e) = selftest::run(std::path::Path::new(&out), args.get(2).map(std::path::PathBuf::from)) {
            eprintln!("selftest failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }
    let open = args.first().map(std::path::PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Drifting VOID")
            .with_inner_size([1400.0, 900.0])
            .with_min_inner_size([700.0, 450.0])
            .with_drag_and_drop(true)
            .with_icon(window_icon()),
        multisampling: 4,
        // wgpu: the canvas and glass draw on the GPU renderer (gpu.rs, glass_gpu.rs)
        renderer: eframe::Renderer::Wgpu,
        // the export renders on this device too: the adapter's full texture size
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
            wgpu_setup: eframe::egui_wgpu::WgpuSetup::CreateNew(eframe::egui_wgpu::WgpuSetupCreateNew {
                power_preference: eframe::wgpu::PowerPreference::HighPerformance,
                device_descriptor: std::sync::Arc::new(|adapter| eframe::wgpu::DeviceDescriptor {
                    label: Some("drifting void"),
                    required_limits: eframe::wgpu::Limits { max_texture_dimension_2d: adapter.limits().max_texture_dimension_2d, ..eframe::wgpu::Limits::default() },
                    ..Default::default()
                }),
                instance_descriptor: {
                    let mut d = eframe::egui_wgpu::WgpuSetupCreateNew::without_display_handle().instance_descriptor;
                    d.backends = gpu::backends();
                    d
                },
                ..eframe::egui_wgpu::WgpuSetupCreateNew::without_display_handle()
            }),
            // DV_NO_VSYNC: frames as fast as they come; DV_LATENCY=n: frames
            // queued ahead (frame-rate checks)
            surface: eframe::egui_wgpu::SurfaceConfig {
                present_mode: match std::env::var("DV_PRESENT").as_deref() {
                    Ok("mailbox") => eframe::wgpu::PresentMode::Mailbox,
                    Ok("immediate") => eframe::wgpu::PresentMode::Immediate,
                    Ok("fifo") => eframe::wgpu::PresentMode::Fifo,
                    _ if std::env::var_os("DV_NO_VSYNC").is_some() => eframe::wgpu::PresentMode::AutoNoVsync,
                    _ => eframe::wgpu::PresentMode::AutoVsync,
                },
                desired_maximum_frame_latency: Some(std::env::var("DV_LATENCY").ok().and_then(|v| v.parse().ok()).unwrap_or(1)),
            },
            ..Default::default()
        },
        ..Default::default()
    };
    eframe::run_native("Drifting VOID", options, Box::new(move |cc| Ok(Box::new(app::App::new(cc, open)))))
}

fn window_icon() -> egui::IconData {
    let img = image::load_from_memory(include_bytes!("../assets/icon-256.png")).expect("bundled icon").to_rgba8();
    let (w, h) = img.dimensions();
    egui::IconData { rgba: img.into_raw(), width: w, height: h }
}

/// Release builds have no console, so panics go to %APPDATA%/DriftingVOID/crash.log.
fn install_crash_log() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(dir) = std::env::var_os("APPDATA").map(|a| std::path::Path::new(&a).join("DriftingVOID")) {
            let _ = std::fs::create_dir_all(&dir);
            let bt = std::backtrace::Backtrace::force_capture();
            let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            let rel = match env!("DV_RELEASE") { "" => String::new(), r => format!(" (release {r})") };
            let text = format!("[{secs}] v{}{rel} {info}\n{bt}\n\n", env!("CARGO_PKG_VERSION"));
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("crash.log")) {
                let _ = f.write_all(text.as_bytes());
            }
        }
        prev(info);
    }));
}
