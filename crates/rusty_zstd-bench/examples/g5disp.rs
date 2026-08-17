//! Is Gate 4/5's specialisation CONTENT- or SIZE-dependent?
//! Three independent ABBA runs per corpus. A dispatch requires a STABLE sign;
//! signs that disagree across runs are noise, not a signal.
fn best(src: &[u8], spec: bool, n: usize) -> f64 {
    rusty_zstd::set_dfast_spec_arm(spec);
    let mut b = f64::MAX;
    for _ in 0..n {
        let t = std::time::Instant::now();
        let _ = rusty_zstd::compress(src, 3).unwrap();
        let e = t.elapsed().as_secs_f64()*1000.0;
        if e < b { b = e; }
    }
    b
}
fn main() {
    let ids = ["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m",
               "mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
    println!("{:<14}{:>9}{:>9}{:>9}{:>9}   verdict", "corpus", "run1 %", "run2 %", "run3 %", "MiB");
    let (mut stable_spec, mut stable_gen, mut unstable) = (0,0,0);
    for id in ids {
        let Ok(full) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let src = &full[..full.len().min(8*1024*1024)];
        let mut d = [0.0f64;3];
        for r in 0..3 {
            let g1=best(src,false,7); let s1=best(src,true,7);
            let s2=best(src,true,7);  let g2=best(src,false,7);
            let (g,s)=(g1.min(g2), s1.min(s2));
            d[r]=100.0*(s-g)/g;
        }
        let neg = d.iter().filter(|&&x| x < -0.5).count();
        let pos = d.iter().filter(|&&x| x >  0.5).count();
        let v = if neg==3 {stable_spec+=1; "STABLE: spec wins"}
                else if pos==3 {stable_gen+=1; "STABLE: generic wins"}
                else {unstable+=1; "unstable / within noise"};
        println!("{id:<14}{:>9.2}{:>9.2}{:>9.2}{:>9.1}   {v}", d[0],d[1],d[2], src.len() as f64/1048576.0);
    }
    println!("\nstable-spec {stable_spec}   stable-generic {stable_gen}   unstable {unstable}  (of 18)");
    println!("{}", if stable_gen>0 && stable_spec>0 { "SIGN FLIP -> dispatch candidate" }
                   else { "no stable sign flip -> CONSTANT (empirically, not just structurally)" });
    rusty_zstd::set_dfast_spec_arm(true);
}
