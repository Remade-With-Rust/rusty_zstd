const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main(){
    let srcs: Vec<(&str,Vec<u8>)> = IDS.iter().filter_map(|id|
        std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
            .ok().map(|f|{let n=f.len().min(8<<20);(*id,f[..n].to_vec())})).collect();
    // ORIGINAL: constant ON, no gate
    std::env::set_var("RZSTD_REPMIN","0.0");
    let (mut a_sz,mut a_pos)=(0usize,0u64);
    let mut base=vec![];
    for (_,s) in &srcs { let _=rusty_zstd::take_dfast_rep_blocks();
        let n=rusty_zstd::compress(s,3).unwrap().len(); let (_,_,p)=rusty_zstd::take_dfast_rep_blocks();
        base.push(n); a_sz+=n; a_pos+=p; }
    std::env::remove_var("RZSTD_REPMIN");
    let (mut b_sz,mut b_pos)=(0usize,0u64); let mut worst=0.0f64; let mut wc="";
    for (i,(id,s)) in srcs.iter().enumerate() { let _=rusty_zstd::take_dfast_rep_blocks();
        let z=rusty_zstd::compress(s,3).unwrap(); let (_,_,p)=rusty_zstd::take_dfast_rep_blocks();
        assert_eq!(rusty_zstd::decompress(&z).unwrap(),*s,"{id}");
        b_sz+=z.len(); b_pos+=p;
        let d=100.0*(z.len() as f64-base[i] as f64)/base[i] as f64;
        if d>worst {worst=d; wc=id;} }
    println!("  rep probe positions {a_pos} -> {b_pos}   ({:.1}% removed)", 100.0*(a_pos-b_pos) as f64/a_pos as f64);
    println!("  total size {:+.4}%   worst regression +{worst:.3}% ({wc})", 100.0*(b_sz as f64-a_sz as f64)/a_sz as f64);
}
