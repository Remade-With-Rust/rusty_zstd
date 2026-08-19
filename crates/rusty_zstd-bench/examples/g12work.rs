//! GATE 12 @ L3: price the interior back-fill in the unit its cost is paid in.
//! Cost = interior inserts (2 hash+store each). Benefit = main-loop positions
//! NOT visited, each of which costs 2 hashes plus up to 3 probes.
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(3);
    println!("{:>7}{:>13}{:>13}{:>13}{:>12}{:>11}","stride","size","mainloop pos","interior ins","net work","size %");
    let (mut b_sz,mut b_mm)=(0i64,0u64);
    for s in [0usize,16,8,4,2,1]{
        let (mut sz,mut mm,mut ins)=(0i64,0u64,0u64);
        for id in IDS{
            let Some(full)=load(id) else{continue};
            let src=&full[..full.len().min(2<<20)];
            rusty_zstd::set_dfast_fill_stride_arm(s);
            let _=rusty_zstd::take_mm(); let _=rusty_zstd::take_dfast_fill();
            sz+=rusty_zstd::compress(src,lvl).unwrap().len() as i64;
            mm+=rusty_zstd::take_mm().0;
            ins+=rusty_zstd::take_dfast_fill();
        }
        if s==0 {b_sz=sz; b_mm=mm;
            println!("{s:>7}{sz:>13}{mm:>13}{ins:>13}{:>12}{:>11}","-","(today)");
        } else {
            let saved=b_mm as i64 - mm as i64;   // positions not visited
            let net=ins as i64 - saved;          // >0 = net MORE work
            println!("{s:>7}{sz:>13}{mm:>13}{ins:>13}{net:>+12}{:>+10.4}%",
                100.0*(sz-b_sz) as f64/b_sz as f64);
        }
    }
    rusty_zstd::set_dfast_fill_stride_arm(0);
}
