//! END-TO-END: what the faster XXH64 buys on real decode. Buffer reused
//! (4.79's lesson), ABBA-paired, null arm.
use std::time::Instant;
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","versions-16m","nci","xml","jsonlog-16m","dickens","mozilla","samba","webster","mr","sao","x-ray","osdb","reymont","ooffice","smallmsg-8m"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn dt(z:&[u8],dst:&mut Vec<u8>,avx:bool,n:usize)->f64{
    rusty_zstd::set_xxh_avx2_arm(avx);
    let mut b=f64::MAX;
    for _ in 0..n{
        dst.clear();
        let s=Instant::now();
        let k=std::hint::black_box(rusty_zstd::decompress_into(dst,std::hint::black_box(z)).unwrap());
        let e=s.elapsed().as_secs_f64();
        std::hint::black_box(k);
        if e<b{b=e;}
    }
    b
}
fn main(){
    println!("DECODE end-to-end, 8 MiB, best-of-13 x ABBA x5. negative = AVX2 hash FASTER\n");
    println!("{:<14}{:>9}{:>10}","corpus","null","avx2");
    let (mut sn,mut sa,mut c)=(0.0,0.0,0.0);
    for id in IDS{
        let Some(f)=load(id) else{continue};
        let src=&f[..f.len().min(8<<20)];
        let z=rusty_zstd::compress(src,3).unwrap();
        let mut dst=Vec::with_capacity(src.len()+4096);
        let _=rusty_zstd::decompress_into(&mut dst,&z).unwrap();
        rusty_zstd::set_xxh_avx2_arm(true);
        assert_eq!(rusty_zstd::decompress(&z).unwrap(), src, "{id} roundtrip");
        let (mut a,mut av,mut nn)=(f64::MAX,f64::MAX,f64::MAX);
        for _ in 0..5{
            a=a.min(dt(&z,&mut dst,false,13));
            av=av.min(dt(&z,&mut dst,true,13));
            nn=nn.min(dt(&z,&mut dst,false,13));
        }
        let dn=100.0*(nn-a)/a; let da=100.0*(av-a)/a;
        sn+=dn.abs(); sa+=da; c+=1.0;
        println!("{id:<14}{dn:>+8.2}%{da:>+9.2}%");
    }
    println!("\nmean |null| {:.2}%   mean AVX2 {:+.2}%",sn/c,sa/c);
    rusty_zstd::set_xxh_avx2_arm(true);
}
