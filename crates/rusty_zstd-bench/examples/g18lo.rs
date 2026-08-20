//! 4.72 dispatch: complete work ledger + size, per corpus and total.
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn run(id:&str,on:bool)->(i64,u64,u64){
    rusty_zstd::set_pair_lo_arm(if on {0.71} else {0.0});
    let f=load(id).unwrap();
    let src=&f[..f.len().min(8<<20)];
    let _=rusty_zstd::take_mm(); let _=rusty_zstd::take_pair_stats();
    let z=rusty_zstd::compress(src,1).unwrap();
    assert_eq!(rusty_zstd::decompress(&z).unwrap(),src,"{id} roundtrip");
    (z.len() as i64, rusty_zstd::take_mm().0, rusty_zstd::take_pair_stats().0)
}
fn main(){
    println!("4.72 pair_gain_lo dispatch -- complete ledger\n");
    println!("{:<14}{:>10}{:>13}{:>12}{:>12}","corpus","size %","d positions","d pair","NET ops");
    let (mut ts0,mut ts1,mut tw0,mut tw1)=(0i64,0i64,0i64,0i64);
    for id in IDS{
        if load(id).is_none(){continue;}
        let (s0,p0,q0)=run(id,false);
        let (s1,p1,q1)=run(id,true);
        let w0=p0 as i64+q0 as i64; let w1=p1 as i64+q1 as i64;
        ts0+=s0; ts1+=s1; tw0+=w0; tw1+=w1;
        if s0==s1 && w0==w1 {continue;}
        println!("{id:<14}{:>+9.3}%{:>13}{:>12}{:>12}",
            100.0*(s1-s0) as f64/s0 as f64, p1 as i64-p0 as i64, q1 as i64-q0 as i64, w1-w0);
    }
    println!("\nTOTAL size {:+} bytes ({:+.4}%)   NET search ops {:+} ({:+.2}%)",
        ts1-ts0, 100.0*(ts1-ts0) as f64/ts0 as f64, tw1-tw0, 100.0*(tw1-tw0) as f64/tw0 as f64);
    rusty_zstd::set_pair_lo_arm(f32::NAN);
}
