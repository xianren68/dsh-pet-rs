//! 一次性小工具：统计一张 PNG 里**不透明像素**的平均颜色和饱和度。
//!
//! 用法：`cargo run --release --example colorstat -- <图.png>`
//!
//! 只算 alpha > 200 的像素，透明区域会被完全排除 —— 否则画面里大片透明
//! （RGB 往往是 0 或残留色）会把均值拉偏，得出来的数根本没法用。

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = args.first() else {
        eprintln!("用法: colorstat <图.png>");
        std::process::exit(2);
    };
    let img = match image::open(path) {
        Ok(i) => i.to_rgba8(),
        Err(e) => {
            eprintln!("读不了 {path}: {e}");
            std::process::exit(1);
        }
    };

    let mut n: u64 = 0;
    let (mut sr, mut sg, mut sb) = (0u64, 0u64, 0u64);
    let mut sat_sum = 0f64;
    let mut lum_sum = 0f64;

    for p in img.pixels() {
        if p[3] < 200 {
            continue;
        }
        let (r, g, b) = (p[0] as f64, p[1] as f64, p[2] as f64);
        n += 1;
        sr += p[0] as u64;
        sg += p[1] as u64;
        sb += p[2] as u64;

        let mx = r.max(g).max(b);
        let mn = r.min(g).min(b);
        sat_sum += if mx > 0.0 { (mx - mn) / mx } else { 0.0 };
        // 感知亮度，和 signalstats 的 Y 大致同量级
        lum_sum += 0.299 * r + 0.587 * g + 0.114 * b;
    }

    if n == 0 {
        println!("{path}: 没有不透明像素");
        return;
    }
    let nf = n as f64;
    println!(
        "{:<28} n={:<7} R={:<6.1} G={:<6.1} B={:<6.1} Y={:<6.2} S={:.4}",
        path,
        n,
        sr as f64 / nf,
        sg as f64 / nf,
        sb as f64 / nf,
        lum_sum / nf,
        sat_sum / nf
    );
}
