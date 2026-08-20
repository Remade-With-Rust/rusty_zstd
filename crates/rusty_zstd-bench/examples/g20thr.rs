//! Where does the fused win actually clear its null? 2 / 4 / 6 / 8 MiB.
use std::time::Instant;
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","versions-16m","mozilla","webster","dickens","nci"];
const MB:&[usize]=&[2,4,6,8];
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
    println!("FUSED vs separate at the crossover. best-of-11 x ABBA x5\n");
    print!("{:<14}"," corpus"); for m in MB{ print!("{:>15}",format!("{m} MiB (null)")); } println!();
    let mut agg=vec![(0.0f64,0.0f64,0.0f64);MB.len()];
    for id in IDS{
        let Some(f)=load(id) else{continue};
        print!("{id:<14}");
        for (i,m) in MB.iter().enumerate(){
            let n=m<<20;
            if f.len()<n { print!("{:>15}","-"); continue; }
            let z=rusty_zstd::compress(&f[..n],3).unwrap();
            let (mut a,mut fu,mut nn)=(f64::MAX,f64::MAX,f64::MAX);
            for _ in 0..5{
                a=a.min(dt(&z,false,11));
                fu=fu.min(dt(&z,true,11));
                nn=nn.min(dt(&z,false,11));
            }
            let dn=100.0*(nn-a)/a; let df=100.0*(fu-a)/a;
            agg[i].0+=df; agg[i].1+=dn.abs(); agg[i].2+=1.0;
            print!("{:>15}",format!("{df:+.2}% ({dn:+.1}%)"));
        }
        println!();
    }
    print!("{:<14}","MEAN");
    for (s,n,c) in &agg{
        if *c>0.0 {
            let (m,nl)=(s/c,n/c);
            print!("{:>15}",format!("{m:+.2}% ({nl:.1}%)"));
        } else { print!("{:>15}","-"); }
    }
    println!("\n\nMEAN row: effect (its null). Ship the threshold where effect >= 2x null.");
    rusty_zstd::set_ck_stream_arm(false);
}
