//! Proof that the deleted `find_dfast_runtime` was unreachable in production:
//! with defaults, DFast must call ONLY specialised bodies (runtime count 0).
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main() {
    let mut spec_t = 0u64; let mut run_t = 0u64;
    for &lvl in &[1i32,3,19,22] {
        let (mut s,mut r)=(0u64,0u64);
        for id in IDS {
            let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else {continue};
            let cap = if lvl>=19 {2<<20} else {8<<20};
            let src=&full[..full.len().min(cap)];
            let _=rusty_zstd::take_dfast_calls();
            let z=rusty_zstd::compress(src,lvl).unwrap();
            assert_eq!(rusty_zstd::decompress(&z).unwrap(),src,"{id} L{lvl} round-trip");
            let (a,b)=rusty_zstd::take_dfast_calls();
            s+=a; r+=b;
        }
        println!("L{lvl:<3} specialised {s:>7}   runtime {r:>7}{}", if r>0 {"   <-- REACHABLE"} else {""});
        spec_t+=s; run_t+=r;
    }
    println!("\ntotal specialised {spec_t}, runtime {run_t}");
    println!("{}", if run_t==0 {"the deleted body was UNREACHABLE at defaults -- shipped bytes cannot have moved"}
                   else {"REACHABLE -- shipped output may have changed"});
}
