//! GATE 20 STEP 2, the LEGITIMATE dispatch: not WHETHER to verify (correctness)
//! but WHERE the hash runs. Fused-per-block vs one pass after decode compute the
//! SAME XXH64 over the same bytes, so this is pure speed with no correctness trade.
//! The fused arm was rejected on aggregate -- but the corpora where the checksum
//! is 35-42% of decode are exactly where its locality argument should win.
use std::time::Instant;
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","versions-16m","nci","xml","jsonlog-16m","smallmsg-8m","mr","dickens","mozilla","samba","webster","sao","x-ray","osdb"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn dt(z:&[u8],fused:bool,n:usize)->f64{
    rusty_zstd::set_ck_stream_arm(fused);
    let mut b=f64::MAX;
    for _ in 0..n{
        let s=Instant::now();
        let v=std::hint::black_box(rusty_zstd::decompress(std::hint::black_box(z)).unwrap());
        let e=s.elapsed().as_secs_f64();
        std::hint::black_box(v.len());
        if e<b{b=e;}
    }
    b
}
fn main(){
    println!("GATE 20: FUSED checksum vs separate pass. negative = fused is FASTER");
    println!("best-of-9 x ABBA x4, null arm. identical output REQUIRED.\n");
    println!("{:<14}{:>9}{:>9}{:>10}{:>10}","corpus","ratio","null","fused","verdict");
    let (mut sn,mut sf,mut c)=(0.0,0.0,0.0);
    for id in IDS{
        let Some(f)=load(id) else{continue};
        let src=&f[..f.len().min(8<<20)];
        let z=rusty_zstd::compress(src,3).unwrap();
        // both arms must decode identically AND still catch corruption
        rusty_zstd::set_ck_stream_arm(true);
        assert_eq!(rusty_zstd::decompress(&z).unwrap(), src, "{id} fused roundtrip");
        let mut bad=z.clone(); let n=bad.len(); bad[n-1]^=0xFF;
        assert!(rusty_zstd::decompress(&bad).is_err(), "{id} fused must reject corruption");
        rusty_zstd::set_ck_stream_arm(false);
        assert_eq!(rusty_zstd::decompress(&z).unwrap(), src, "{id} separate roundtrip");
        assert!(rusty_zstd::decompress(&bad).is_err(), "{id} separate must reject corruption");
        let (mut a,mut fu,mut nn)=(f64::MAX,f64::MAX,f64::MAX);
        for _ in 0..4{
            a=a.min(dt(&z,false,9));
            fu=fu.min(dt(&z,true,9));
            nn=nn.min(dt(&z,false,9));
        }
        let dn=100.0*(nn-a)/a; let df=100.0*(fu-a)/a;
        sn+=dn.abs(); sf+=df; c+=1.0;
        let v=if df < -3.0 {"FUSED WINS"} else if df>3.0 {"separate"} else {"tie"};
        println!("{id:<14}{:>9.4}{dn:>+8.2}%{df:>+9.2}%{v:>10}",z.len() as f64/src.len() as f64);
    }
    println!("\nmean |null| {:.2}%   mean fused {:+.2}%",sn/c,sf/c);
    rusty_zstd::set_ck_stream_arm(false);
}
