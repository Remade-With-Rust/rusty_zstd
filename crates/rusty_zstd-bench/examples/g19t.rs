//! GATE 19 @ L1: is the block-size time cost REAL? ABBA-paired, with a null arm
//! (128 vs 128) so a +2% reading can be told from noise.
use std::time::Instant;
const IDS:&[&str]=&["mr","mozilla","sao","xml","ooffice","samba","osdb","x-ray","smallmsg-8m","nci"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn t(src:&[u8],kb:usize,n:usize)->f64{
    unsafe{ std::env::set_var("RZSTD_BLOCK_KB", kb.to_string()); }
    let mut b=f64::MAX;
    for _ in 0..n{
        let s=Instant::now();
        let z=std::hint::black_box(rusty_zstd::compress(std::hint::black_box(src),1).unwrap());
        let e=s.elapsed().as_secs_f64();
        std::hint::black_box(z.len());
        if e<b{b=e;}
    }
    b
}
fn main(){
    println!("L1 encode, 8 MiB, best-of-7 x ABBA x4. negative = FASTER than 128 KiB\n");
    println!("{:<12}{:>9}{:>9}{:>9}{:>9}","corpus","null","96KB","64KB","32KB");
    let (mut sn,mut s96,mut s64,mut s32,mut c)=(0.0,0.0,0.0,0.0,0.0);
    for id in IDS{
        let Some(f)=load(id) else{continue};
        let src=&f[..f.len().min(8<<20)];
        let (mut a,mut nn,mut x96,mut x64,mut x32)=(f64::MAX,f64::MAX,f64::MAX,f64::MAX,f64::MAX);
        for _ in 0..4{
            a=a.min(t(src,128,7));
            x96=x96.min(t(src,96,7));
            x64=x64.min(t(src,64,7));
            x32=x32.min(t(src,32,7));
            nn=nn.min(t(src,128,7));
        }
        let d=|v:f64|100.0*(v-a)/a;
        sn+=d(nn).abs(); s96+=d(x96); s64+=d(x64); s32+=d(x32); c+=1.0;
        println!("{id:<12}{:>+8.2}%{:>+8.2}%{:>+8.2}%{:>+8.2}%",d(nn),d(x96),d(x64),d(x32));
    }
    println!("\nmean |null| {:.2}%   96KB {:+.2}%   64KB {:+.2}%   32KB {:+.2}%",
        sn/c,s96/c,s64/c,s32/c);
    unsafe{ std::env::remove_var("RZSTD_BLOCK_KB"); }
}
