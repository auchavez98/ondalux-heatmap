//! Runs one WiFi scan with the native backend and prints the results.
//!
//! Usage: `cargo run -p ondalux-scan --example scan`

use ondalux_core::geometry::Vec2;

fn main() {
    let mut scanner = match ondalux_scan::platform_scanner() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    println!("Backend: {}", scanner.name());
    let start = std::time::Instant::now();
    match scanner.scan(Vec2::default()) {
        Ok(mut readings) => {
            readings.sort_by(|a, b| b.rssi.0.total_cmp(&a.rssi.0));
            println!(
                "{} BSS in {:.1} s\n{:<32} {:<17} {:>6} {:>6} {:>4}  Security",
                readings.len(),
                start.elapsed().as_secs_f32(),
                "SSID",
                "BSSID",
                "dBm",
                "MHz",
                "Ch"
            );
            for r in readings {
                let ch = r.channel().map_or("?".to_owned(), |(_, n)| n.to_string());
                println!(
                    "{:<32} {:<17} {:>6.0} {:>6} {:>4}  {}",
                    if r.ssid.is_empty() {
                        "(hidden)"
                    } else {
                        &r.ssid
                    },
                    r.bssid,
                    r.rssi.0,
                    r.frequency_mhz,
                    ch,
                    r.security
                );
            }
        }
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
