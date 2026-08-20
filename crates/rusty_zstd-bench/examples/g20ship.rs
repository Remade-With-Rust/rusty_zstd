//! 4.79 shipped: output-size-dispatched checksum. Correctness FIRST, then speed.
use std::time::Instant;
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","versions-16m","nci","xml","dickens","mozilla","mr","samba","webster","sao","x-ray","osdb","jsonlog-16m","smallmsg-8m","reymont","ooffice"];
const MB:&[usize]=&[1,2,8,32];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn dt(z:&[u8],arm:usize,n:usize)->f64{
    rusty_zstd::set_ck_fuse_arm(arm);
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
    // ---- CORRECTNESS GATE: every corpus, every size, both arms ----
    let (mut ok,mut rej)=(0,0);
    for id in IDS{
        let Some(f)=load(id) else{continue};
        for m in MB{
            let n=m<<20; if f.len()<n {continue;}
            let src=&f[..n];
            for lv in [1,3]{
                let z=rusty_zstd::compress(src,lv).unwrap();
                for arm in [usize::MAX, 0]{
                    rusty_zstd::set_ck_fuse_arm(arm);
                    assert_eq!(rusty_zstd::decompress(&z).unwrap(), src, "{id} {m}MiB L{lv} arm{arm}");
                    ok+=1;
                    let mut bad=z.clone(); let k=bad.len(); bad[k-1]^=0xFF;
                    assert!(rusty_zstd::decompress(&bad).is_err(), "{id} {m}MiB L{lv} arm{arm} must REJECT corruption");
                    rej+=1;
                }
            }
        }
    }
    println!("CORRECTNESS: {ok} round-trips exact, {rej} corrupt frames rejected, both arms\n");

    println!("4.79 vs pre-4.79. negative = 4.79 FASTER. best-of-9 x ABBA x4\n");
    print!("{:<14}"," corpus"); for m in MB{ print!("{:>15}",format!("{m} MiB (null)")); } println!();
    let mut agg=vec![(0.0f64,0.0f64,0.0f64);MB.len()];
    for id in IDS{
        let Some(f)=load(id) else{continue};
        print!("{id:<14}");
        for (i,m) in MB.iter().enumerate(){
            let n=m<<20;
            if f.len()<n { print!("{:>15}","-"); continue; }
            let z=rusty_zstd::compress(&f[..n],3).unwrap();
            let (mut a,mut d,mut nn)=(f64::MAX,f64::MAX,f64::MAX);
            for _ in 0..4{
                a=a.min(dt(&z,usize::MAX,9));
                d=d.min(dt(&z,0,9));
                nn=nn.min(dt(&z,usize::MAX,9));
            }
            let dn=100.0*(nn-a)/a; let dd=100.0*(d-a)/a;
            agg[i].0+=dd; agg[i].1+=dn.abs(); agg[i].2+=1.0;
            print!("{:>15}",format!("{dd:+.2}% ({dn:+.1}%)"));
        }
        println!();
    }
    print!("{:<14}","MEAN");
    for (s,n,c) in &agg{
        if *c>0.0 { print!("{:>15}",format!("{:+.2}% ({:.1}%)",s/c,n/c)); } else { print!("{:>15}","-"); }
    }
    println!();
    rusty_zstd::set_ck_fuse_arm(0);
}
