//! GATE 9 @ L1 RE-MEASURED with work parity: step 3/4 now get the same
//! HLOG+STEP specialisation as 1/2. Also reports the DETERMINISTIC probe count,
//! which no amount of machine load can distort.
const IDS: &[&str] = &["mr","dickens","sao","ooffice","mozilla","samba","osdb","webster","reymont","nci","xml","smallmsg-8m","jsonlog-16m","x-ray"];
fn ms(src:&[u8],st:usize,n:usize)->f64{ rusty_zstd::set_step0_arm(st); let mut b=f64::MAX;
    for _ in 0..n { let t=std::time::Instant::now(); let _=rusty_zstd::compress(src,1).unwrap();
        let e=t.elapsed().as_secs_f64()*1000.0; if e<b{b=e;} } b }
fn paired(src:&[u8],a:usize,b:usize)->f64{ let mut d=vec![];
    for _ in 0..3 { let a1=ms(src,a,5); let b1=ms(src,b,5); let b2=ms(src,b,5); let a2=ms(src,a,5);
        d.push(0.5*(100.0*(b1-a1)/a1+100.0*(b2-a2)/a2)); }
    d.sort_by(|x,y| x.partial_cmp(y).unwrap()); d[1] }
fn main(){
    println!("{:<14}{:>8}{:>9}{:>9}{:>10}", "corpus","null","sz s3","t s3","probes s3");
    let (mut nn,mut ns,mut nt,mut c)=(0.0,0.0,0.0,0.0);
    for id in IDS {
        let Ok(full)=std::fs::read(format!("corpora/data/silesia/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/generated/{id}"))) else {continue};
        let src=&full[..full.len().min(8<<20)];
        rusty_zstd::set_step0_arm(2);
        let _=rusty_zstd::take_mm();
        let s2=rusty_zstd::compress(src,1).unwrap().len() as f64;
        let (p2,_)=rusty_zstd::take_mm();
        rusty_zstd::set_step0_arm(3);
        let _=rusty_zstd::take_mm();
        let z3=rusty_zstd::compress(src,1).unwrap();
        let (p3,_)=rusty_zstd::take_mm();
        assert_eq!(rusty_zstd::decompress(&z3).unwrap(),src,"{id}");
        let dp = if p2>0 {100.0*(p3 as f64-p2 as f64)/p2 as f64} else {0.0};
        let null=paired(src,2,2); let t3=paired(src,2,3);
        println!("{id:<14}{null:>7.2}%{:>8.2}%{t3:>8.2}%{dp:>9.1}%", 100.0*(z3.len() as f64-s2)/s2);
        nn+=null.abs(); ns+=100.0*(z3.len() as f64-s2)/s2; nt+=t3; c+=1.0;
    }
    println!("\nmean |null| {:.2}%  |  step3 (work-parity): size {:+.2}%  time {:+.2}%", nn/c, ns/c, nt/c);
    rusty_zstd::set_step0_arm(2);
}
