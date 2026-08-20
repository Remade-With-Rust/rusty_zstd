//! 4.77 shipped: size-dispatched Fast ladder. OFF arm = pre-4.77 (always on).
const IDS:&[&str]=&["mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray","text-32m","jsonlog-16m","smallmsg-8m","versions-16m","incomp-32m","zeros-32m"];
const CAPS:&[usize]=&[1,2,4,8];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    println!("4.77 size-dispatched Fast ladder vs pre-4.77 (always on)\n");
    println!("{:<14}{:>10}{:>10}{:>10}{:>10}","corpus","1 MiB","2 MiB","4 MiB","8 MiB");
    let mut tot=vec![(0i64,0i64);CAPS.len()];
    let mut worst=(0.0f64,String::new());
    for id in IDS{
        let Some(f)=load(id) else{continue};
        print!("{id:<14}");
        for (i,mb) in CAPS.iter().enumerate(){
            let src=&f[..f.len().min(mb<<20)];
            if src.len()<(mb<<20) && *mb>1 { print!("{:>10}","-"); continue; }
            rusty_zstd::set_g5_fast_len_arm(usize::MAX);   // pre-4.77
            let a=rusty_zstd::compress(src,1).unwrap().len() as i64;
            rusty_zstd::set_g5_fast_len_arm(0);            // shipped
            let b=rusty_zstd::compress(src,1).unwrap().len() as i64;
            assert_eq!(rusty_zstd::decompress(&rusty_zstd::compress(src,1).unwrap()).unwrap(),src);
            tot[i].0+=a; tot[i].1+=b;
            let d=100.0*(b-a) as f64/a as f64;
            if d>worst.0 {worst=(d,format!("{id}@{mb}MiB"));}
            print!("{d:>9.3}%");
        }
        println!();
    }
    print!("{:<14}","TOTAL");
    for (a,b) in &tot{ print!("{:>9.4}%",100.0*(b-a) as f64/ *a as f64); }
    println!("\n\nworst cell: {} at {:+.3}%   (negative = 4.77 is SMALLER)",worst.1,worst.0);
    rusty_zstd::set_g5_fast_len_arm(0);
}
