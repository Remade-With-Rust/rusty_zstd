//! GATE 20, THE FULL GRID. Every earlier run was at 8 MiB only. The fused-vs-
//! separate question is about CACHE RESIDENCY: the separate pass re-reads the
//! decoded output, which is cache-hot when small and a COLD MEMORY RE-READ when
//! large. So sweep OUTPUT SIZE, which is the axis nobody varied.
use std::time::Instant;
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","versions-16m","nci","xml","dickens","mozilla"];
const MB:&[usize]=&[1,2,8,32];
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
    println!("FUSED vs separate checksum, by OUTPUT SIZE. negative = fused FASTER");
    println!("best-of-9 x ABBA x4, null arm per cell\n");
    print!("{:<14}","corpus");
    for m in MB{ print!("{:>16}",format!("{m} MiB (null)")); }
    println!();
    let mut agg=vec![(0.0f64,0.0f64,0.0f64);MB.len()];
    for id in IDS{
        let Some(f)=load(id) else{continue};
        print!("{id:<14}");
        for (i,m) in MB.iter().enumerate(){
            let n=m<<20;
            if f.len()<n { print!("{:>16}","-"); continue; }
            let src=&f[..n];
            let z=rusty_zstd::compress(src,3).unwrap();
            let (mut a,mut fu,mut nn)=(f64::MAX,f64::MAX,f64::MAX);
            for _ in 0..4{
                a=a.min(dt(&z,false,9));
                fu=fu.min(dt(&z,true,9));
                nn=nn.min(dt(&z,false,9));
            }
            let dn=100.0*(nn-a)/a; let df=100.0*(fu-a)/a;
            agg[i].0+=df; agg[i].1+=dn.abs(); agg[i].2+=1.0;
            print!("{:>16}",format!("{df:+.2}% ({dn:+.1}%)"));
        }
        println!();
    }
    print!("{:<14}","MEAN");
    for (s,n,c) in &agg{
        if *c>0.0 { print!("{:>16}",format!("{:+.2}% ({:.1}%)",s/c,n/c)); } else { print!("{:>16}","-"); }
    }
    println!("\n\n(cell = fused delta, with that cell's own null in parentheses)");
    rusty_zstd::set_ck_stream_arm(false);
}
