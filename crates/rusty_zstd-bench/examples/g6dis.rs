//! GATE 6 SPEED DISPATCH — does the `pair_gain` EARNING term recover the time
//! the pair search costs, without giving back the size it won?
//!
//!   A  pair OFF                     (constant off)
//!   B  rep_yield only               (what shipped: -5.85% size, +28.9% time)
//!   C  rep_yield AND pair_gain >= T (the earning term)
//!
//! Both size and time are reported against A. PAIRED estimator on the time.
const IDS: &[&str] = &[
    "zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr",
    "ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray",
];
#[derive(Clone, Copy)]
enum Arm { Off, RepOnly, Both(f32) }
fn set(a: Arm) {
    match a {
        Arm::Off => rusty_zstd::set_pair_on_arm(false),
        Arm::RepOnly => { rusty_zstd::set_pair_on_arm(true); rusty_zstd::set_pair_gain_arm(0.0); }
        Arm::Both(t) => { rusty_zstd::set_pair_on_arm(true); rusty_zstd::set_pair_gain_arm(t); }
    }
}
fn ms(src: &[u8], a: Arm, n: usize) -> f64 {
    set(a);
    let mut b = f64::MAX;
    for _ in 0..n {
        let t = std::time::Instant::now();
        let _ = rusty_zstd::compress(src, 1).unwrap();
        let e = t.elapsed().as_secs_f64()*1000.0;
        if e < b { b = e; }
    }
    b
}
fn sz(src: &[u8], a: Arm) -> f64 { set(a); rusty_zstd::compress(src, 1).unwrap().len() as f64 }
/// paired A-B-B-A, mean of the two within-phase deltas
fn paired(src: &[u8], a: Arm, b: Arm) -> f64 {
    let mut d = vec![];
    for _ in 0..3 {
        let a1=ms(src,a,5); let b1=ms(src,b,5); let b2=ms(src,b,5); let a2=ms(src,a,5);
        d.push(0.5*(100.0*(b1-a1)/a1 + 100.0*(b2-a2)/a2));
    }
    d.sort_by(|x,y| x.partial_cmp(y).unwrap());
    d[1]
}
fn main() {
    let t: f32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0.05);
    println!("GATE 6 @ L1 — vs pair OFF.  B = rep_yield only, C = + pair_gain >= {t}\n");
    println!("{:<14}{:>10}{:>10}   {:>10}{:>10}", "corpus", "B size", "B time", "C size", "C time");
    println!("{}", "-".repeat(58));
    let (mut bs,mut bt,mut cs,mut ct,mut n)=(0.0,0.0,0.0,0.0,0.0);
    for id in IDS {
        let Ok(full) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let src = &full[..full.len().min(8*1024*1024)];
        let a = sz(src, Arm::Off);
        let bsz = 100.0*(sz(src,Arm::RepOnly)-a)/a;
        let csz = 100.0*(sz(src,Arm::Both(t))-a)/a;
        let btm = paired(src, Arm::Off, Arm::RepOnly);
        let ctm = paired(src, Arm::Off, Arm::Both(t));
        println!("{id:<14}{bsz:>9.2}%{btm:>9.2}%   {csz:>9.2}%{ctm:>9.2}%");
        bs+=bsz; bt+=btm; cs+=csz; ct+=ctm; n+=1.0;
        // correctness: C must round-trip
        set(Arm::Both(t));
        let z = rusty_zstd::compress(src,1).unwrap();
        assert_eq!(rusty_zstd::decompress(&z).unwrap(), src, "{id} round-trip");
    }
    println!("\nB(shipped) size {:+.2}%  time {:+.2}%", bs/n, bt/n);
    println!("C(earning)  size {:+.2}%  time {:+.2}%", cs/n, ct/n);
    println!("\nC recovers {:.1}% of B's time cost, keeps {:.1}% of its size win",
        100.0*(bt/n-ct/n)/(bt/n), 100.0*(cs/n)/(bs/n));
}
