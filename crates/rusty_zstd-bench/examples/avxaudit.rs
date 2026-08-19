//! AVX2 AUDIT: every site where AVX2 could deploy and does not, side by side.
//! Only >=32B ops qualify -- a 16B copy is already ONE movups, so AVX2 cannot
//! improve it. Ranked by instructions saved = executions x (SSE - AVX2).
const IDS:&[&str]=&["jsonlog-16m","smallmsg-8m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray","text-32m","versions-16m","incomp-32m","zeros-32m"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(3);
    let (mut l32,mut m32,mut l16,mut m16)=(0u64,0u64,0u64,0u64);
    let (mut t2,mut t3,mut t1)=(0u64,0u64,0u64);
    let mut eqw=0u64;
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(2<<20)];
        let _=rusty_zstd::take_dec_copies(); let _=rusty_zstd::take_lit_push();
        let _=rusty_zstd::take_lit_tiers(); let _=rusty_zstd::take_eq_ops();
        let z=rusty_zstd::compress(src,lvl).unwrap();
        let (f1,_s)=rusty_zstd::take_lit_push();
        let (f2,f3)=rusty_zstd::take_lit_tiers();
        let (w,_,_)=rusty_zstd::take_eq_ops();
        t1+=f1; t2+=f2; t3+=f3; eqw+=w;
        let _=rusty_zstd::decompress(&z).unwrap();
        let (a,b,c,d)=rusty_zstd::take_dec_copies();
        l32+=a; m32+=b; l16+=c; m16+=d;
    }
    println!("L{lvl}, 18 corpora, encode + decode\n");
    println!("{:<34}{:>7}{:>14}{:>10}{:>14}","site","width","executions","instr/ex","instr saved");
    let rows: Vec<(&str,usize,u64,i64)> = vec![
        ("decoder match copy",        32, m32, 2),
        ("decoder literal copy",      32, l32, 2),
        ("push_literals tier2",       32, t2,  2),
        ("push_literals tier3",       64, t3,  4),
        ("-- already minimal --",      0, 0,   0),
        ("decoder match copy (16B)",  16, m16, 0),
        ("decoder literal copy (16B)",16, l16, 0),
        ("push_literals tier1 (16B)", 16, t1,  0),
        ("-- already AVX2 --",         0, 0,   0),
        ("count_eq_len wide ops",     32, eqw, 0),
    ];
    let mut tot=0i64;
    for (name,w,ex,save) in &rows{
        if *w==0 { println!("{name}"); continue; }
        let s=*save*(*ex as i64);
        if *save>0 {tot+=s;}
        println!("{name:<34}{w:>6}B{ex:>14}{save:>10}{:>14}", if *save>0 {format!("{s}")} else {"-".into()});
    }
    println!("\nTOTAL instructions AVX2 could remove: {tot}");
}
