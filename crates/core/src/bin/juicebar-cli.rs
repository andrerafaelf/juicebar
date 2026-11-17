//! `juicebar-cli` — print every detected battery and exit. Useful for bug reports.
//!
//! Usage: juicebar-cli [--json] [--verbose] [--demo] [--watch]

use juicebar_core::{Charge, Monitor, MonitorOptions, ProviderOptions};
use std::time::Duration;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |f: &str| args.iter().any(|a| a == f);
    if flag("--help") || flag("-h") {
        println!("juicebar-cli [--json] [--verbose] [--demo] [--watch] [--dump]");
        return;
    }
    if flag("--dump") {
        dump_hid();
        return;
    }
    let opts = MonitorOptions { providers: ProviderOptions { demo: flag("--demo"), ..Default::default() }, ..Default::default() };
    let mut monitor = Monitor::new(opts);

    loop {
        let devices = monitor.poll(true);
        if flag("--json") {
            println!("{}", serde_json::to_string_pretty(&devices).unwrap_or_default());
        } else if devices.is_empty() {
            println!("No devices with battery information found.");
        } else {
            println!("{:<32} {:>5}  {:<11} {:<9} {:<15} KEY", "DEVICE", "LEVEL", "STATE", "LINK", "SOURCE");
            for d in &devices {
                let state = match d.charge {
                    Charge::Charging => "charging",
                    Charge::Full => "full",
                    Charge::Discharging => "discharging",
                    Charge::Unknown => "-",
                };
                let level = if d.step > 1 { format!("~{}%", d.level) } else { format!("{}%", d.level) };
                let link = format!("{:?}", d.transport).to_lowercase();
                println!("{:<32} {:>5}  {:<11} {:<9} {:<15} {}", d.name, level, state, link, d.source, d.key);
                if d.batteries.len() > 1 {
                    for b in &d.batteries {
                        println!("  - {:<28} {:>4}%", b.label.as_deref().unwrap_or("Main"), b.level);
                    }
                }
            }
        }
        if flag("--verbose") {
            eprintln!();
            for t in &monitor.last_timings {
                eprintln!("  {:<16} {:>6} ms  {} found", t.id, t.elapsed.as_millis(), t.found);
            }
        }
        if !flag("--watch") {
            break;
        }
        std::thread::sleep(Duration::from_secs(30));
        println!();
    }
}

/// Print every HID collection with its usage and whether a battery field was found.
fn dump_hid() {
    let Ok(api) = hidapi::HidApi::new() else {
        eprintln!("hidapi unavailable");
        return;
    };
    println!("{:<9} {:<9} {:>3} {:<22} {:<8} PRODUCT", "VID:PID", "PAGE:USE", "IF", "BATTERY FIELD", "BUS");
    for info in api.device_list() {
        let battery = match info.open_device(&api) {
            Err(_) => "(no access)".to_string(),
            Ok(dev) => {
                let mut raw = vec![0u8; 4096];
                match dev.get_report_descriptor(&mut raw) {
                    Err(_) => "(no descriptor)".to_string(),
                    Ok(n) => {
                        let d = juicebar_core::hid_descriptor::parse(&raw[..n]);
                        d.fields
                            .iter()
                            .find(|f| juicebar_core::hid_descriptor::BATTERY_USAGES.contains(&(f.usage_page, f.usage)))
                            .map(|f| format!("{:?} id={} {}b", f.kind, f.report_id, f.bit_size))
                            .unwrap_or_else(|| "-".into())
                    }
                }
            }
        };
        println!(
            "{:04x}:{:04x} {:04x}:{:04x} {:>3} {:<22} {:<8} {}",
            info.vendor_id(),
            info.product_id(),
            info.usage_page(),
            info.usage(),
            info.interface_number(),
            battery,
            format!("{:?}", info.bus_type()),
            info.product_string().unwrap_or("?")
        );
    }
}
