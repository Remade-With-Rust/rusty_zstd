const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main(){
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(19);
    let srcs: Vec<(&str,Vec<u8>)> = IDS.iter().filter_map(|id|
        std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
            .ok().map(|f|{let n=f.len().min(2<<20);(*id,f[..n].to_vec())})).collect();
    // constant ON = the old behaviour
    std::env::set_var("RZSTD_OPT_REP_MIN","-1");
    let (mut a_sz,mut a_p)=(0usize,0u64); let mut base=vec![];
    for (_,s) in &srcs { let _=rusty_zstd::take_opt_rep();
        let n=rusty_zstd::compress(s,lvl).unwrap().len(); let (p,_,_)=rusty_zstd::take_opt_rep();
        base.push(n); a_sz+=n; a_p+=p; }
    std::env::remove_var("RZSTD_OPT_REP_MIN");
    let (mut b_sz,mut b_p)=(0usize,0u64); let mut worst=0.0f64; let mut wc="";
    for (i,(id,s)) in srcs.iter().enumerate() { let _=rusty_zstd::take_opt_rep();
        let z=rusty_zstd::compress(s,lvl).unwrap(); let (p,_,_)=rusty_zstd::take_opt_rep();
        assert_eq!(rusty_zstd::decompress(&z).unwrap(),*s,"{id}");
        b_sz+=z.len(); b_p+=p;
        let d=100.0*(z.len() as f64-base[i] as f64)/base[i] as f64;
        if d>worst {worst=d; wc=id;} }
    println!("L{lvl}  rep probes {a_p} -> {b_p}   ({:.1}% removed)", 100.0*(a_p-b_p) as f64/a_p as f64);
    println!("      total size {:+.4}%   worst regression +{worst:.3}% ({wc})", 100.0*(b_sz as f64-a_sz as f64)/a_sz as f64);
}
