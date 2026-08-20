//! GATE 20 @ L3 -- force_ignore_checksum. STEP 1 (liveness) + the CEILING.
//! Output is identical by construction, so liveness lives on TIME only.
use std::time::Instant;
use rusty_zstd::DecompressOptions;
const IDS:&[&str]=&["mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray","text-32m","jsonlog-16m","smallmsg-8m","versions-16m","incomp-32m","zeros-32m"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn dt(z:&[u8],skip:bool,n:usize)->f64{
    let o=DecompressOptions{ force_ignore_checksum: skip, ..Default::default() };
    let mut b=f64::MAX;
    for _ in 0..n{
        let s=Instant::now();
        let v=std::hint::black_box(rusty_zstd::decompress_with(std::hint::black_box(z),o).unwrap());
        let e=s.elapsed().as_secs_f64();
        std::hint::black_box(v.len());
        if e<b{b=e;}
    }
    b
}
fn main(){
    println!("GATE 20 @ L3 -- the CEILING: what verification costs on decode");
    println!("best-of-9 x ABBA x4, null arm = verify vs verify. positive = verify is SLOWER\n");
    println!("{:<14}{:>10}{:>9}{:>10}{:>11}","corpus","ratio","null","ceiling","identical");
    let (mut sn,mut sc,mut c)=(0.0,0.0,0.0);
    for id in IDS{
        let Some(f)=load(id) else{continue};
        let src=&f[..f.len().min(8<<20)];
        let z=rusty_zstd::compress(src,3).unwrap();
        // correctness: both arms must produce the same bytes
        let a=rusty_zstd::decompress_with(&z,DecompressOptions{force_ignore_checksum:true,..Default::default()}).unwrap();
        let b=rusty_zstd::decompress(&z).unwrap();
        let same=a==b && a==src;
        let (mut v,mut s,mut nn)=(f64::MAX,f64::MAX,f64::MAX);
        for _ in 0..4{
            v=v.min(dt(&z,false,9));   // verify
            s=s.min(dt(&z,true,9));    // skip
            nn=nn.min(dt(&z,false,9)); // null
        }
        let dn=100.0*(nn-v)/v; let dc=100.0*(v-s)/s;
        sn+=dn.abs(); sc+=dc; c+=1.0;
        println!("{id:<14}{:>10.4}{dn:>+8.2}%{dc:>+9.2}%{:>11}",
            z.len() as f64/src.len() as f64, if same {"yes"} else {"NO"});
    }
    println!("\nmean |null| {:.2}%   mean CEILING {:+.2}%",sn/c,sc/c);
}
