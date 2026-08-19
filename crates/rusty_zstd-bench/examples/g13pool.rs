//! Pooled: total L3 encode time over the whole corpus, ABBA-interleaved. One
//! aggregate per arm per round cuts the per-corpus variance that swamped the
//! individual measurements.
const IDS:&[&str]=&["x-ray","dickens","smallmsg-8m","mr","osdb","reymont","webster","sao","jsonlog-16m","samba","mozilla","nci","ooffice","xml"];
fn sweep(srcs:&[Vec<u8>],on:bool)->f64{
    rusty_zstd::set_dfast_litpush_arm(on);
    let t=std::time::Instant::now();
    for s in srcs { let _=rusty_zstd::compress(s,3).unwrap(); }
    t.elapsed().as_secs_f64()*1000.0
}
fn main(){
    let mut srcs=vec![];
    for id in IDS{
        if let Ok(f)=std::fs::read(format!("corpora/data/silesia/{id}"))
            .or_else(|_|std::fs::read(format!("corpora/data/generated/{id}"))) {
            srcs.push(f[..f.len().min(8<<20)].to_vec());
        }
    }
    // warm
    let _=sweep(&srcs,true); let _=sweep(&srcs,false);
    let (mut da,mut dn)=(vec![],vec![]);
    for _ in 0..7 {
        let a1=sweep(&srcs,false); let b1=sweep(&srcs,true);
        let b2=sweep(&srcs,true);  let a2=sweep(&srcs,false);
        da.push(0.5*(100.0*(b1-a1)/a1 + 100.0*(b2-a2)/a2));
        // null: same arm both sides
        let c1=sweep(&srcs,false); let d1=sweep(&srcs,false);
        let d2=sweep(&srcs,false); let c2=sweep(&srcs,false);
        dn.push(0.5*(100.0*(d1-c1)/c1 + 100.0*(d2-c2)/c2));
    }
    da.sort_by(|x,y|x.partial_cmp(y).unwrap());
    dn.sort_by(|x,y|x.partial_cmp(y).unwrap());
    println!("pooled L3 corpus encode, 7 ABBA rounds\n");
    println!("gate 13 deltas: {:?}", da.iter().map(|v|format!("{v:+.2}")).collect::<Vec<_>>());
    println!("null    deltas: {:?}", dn.iter().map(|v|format!("{v:+.2}")).collect::<Vec<_>>());
    println!("\ngate 13 median {:+.2}%   null median {:+.2}%", da[3], dn[3]);
    println!("gate 13 all-negative: {}", da.iter().all(|v|*v<0.0));
    rusty_zstd::set_dfast_litpush_arm(true);
}
