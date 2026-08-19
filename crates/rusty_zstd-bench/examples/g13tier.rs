//! Is the GATE 13 dispatch a per-block WIDTH, or a per-call TIER?
//!
//! A dispatched width must PREDICT the next block's distribution and then serves
//! every run with one constant. A tier reads `n` and picks among several
//! constants -- no signal, no threshold, no warm-up, no misprediction, and both
//! widths stay compile-time constants so both lower to fixed moves.
//!
//! Cost model, per literal append:
//!   fast path : W bytes stored (one or two 16B moves) -- call it W/16 store-ops
//!   slow path : a runtime-length memcpy CALL -- F store-op-equivalents
//! F is the one unknown, so it is SWEPT rather than assumed.
const IDS:&[&str]=&["jsonlog-16m","smallmsg-8m","nci","xml","mozilla","samba","webster","reymont","dickens","osdb","ooffice","mr","sao","x-ray"];
fn cost(h:&[u64;6], tiers:&[usize], f:f64)->f64{
    // bucket upper bounds
    const UB:[usize;6]=[4,8,16,32,64,usize::MAX];
    let mut c=0.0;
    for b in 0..6 {
        let n=h[b] as f64;
        if n==0.0 {continue;}
        // smallest tier that covers this bucket's WORST case
        match tiers.iter().find(|&&t| UB[b]<=t) {
            Some(&t)=> c += n*(t as f64/16.0),
            None    => c += n*f,
        }
    }
    c
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(1);
    let mut hs=vec![];
    for id in IDS{
        let Ok(full)=std::fs::read(format!("corpora/data/silesia/{id}"))
            .or_else(|_|std::fs::read(format!("corpora/data/generated/{id}"))) else{continue};
        let src=&full[..full.len().min(2<<20)];
        let _=rusty_zstd::take_lit_hist();
        let _=rusty_zstd::compress(src,lvl).unwrap();
        hs.push((*id, rusty_zstd::take_lit_hist()));
    }
    let schemes:&[(&str,&[usize])]=&[
        ("width 16 (today)", &[16]),
        ("width 8",          &[8]),
        ("width 32",         &[32]),
        ("TIER 16,32",       &[16,32]),
        ("TIER 16,32,64",    &[16,32,64]),
    ];
    for f in [4.0f64,8.0,16.0,32.0]{
        println!("\n=== slow call costs F = {f} store-ops ===");
        println!("{:<16}{:>12}{:>12}{:>12}{:>12}{:>12}","corpus","w16","w8","w32","tier16/32","tier+64");
        let mut tot=vec![0.0;schemes.len()];
        for (id,h) in &hs{
            let cs:Vec<f64>=schemes.iter().map(|(_,t)|cost(h,t,f)).collect();
            let best=cs.iter().cloned().fold(f64::MAX,f64::min);
            let mark:Vec<String>=cs.iter().map(|c| if (*c-best).abs()<1e-9 {format!("{:.0}*",c)} else {format!("{:.0}",c)}).collect();
            for (i,c) in cs.iter().enumerate(){tot[i]+=c;}
            println!("{id:<16}{:>12}{:>12}{:>12}{:>12}{:>12}",mark[0],mark[1],mark[2],mark[3],mark[4]);
        }
        let best=tot.iter().cloned().fold(f64::MAX,f64::min);
        print!("{:<16}","TOTAL");
        for c in &tot { print!("{:>12}", if (*c-best).abs()<1e-9 {format!("{:.0}*",c)} else {format!("{:.0}",c)}); }
        println!("   (* = best)");
    }
}
