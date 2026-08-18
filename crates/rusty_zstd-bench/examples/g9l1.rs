//! GATE 9 @ L1. The gate is ALIVE (16/18 sizes move). Today's constant is
//! step0=2, with Gate 6's route overriding to 1 on its STEP-1 blocks. Price
//! coarser stepping, and test the SAME mechanism-derived axis that worked at
//! L3: skipping loses a SHORT match and only shifts a LONG one.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn ms(src:&[u8],st:usize,n:usize)->f64{ rusty_zstd::set_step0_arm(st); let mut b=f64::MAX;
    for _ in 0..n { let t=std::time::Instant::now(); let _=rusty_zstd::compress(src,1).unwrap();
        let e=t.elapsed().as_secs_f64()*1000.0; if e<b{b=e;} } b }
fn paired(src:&[u8],a:usize,b:usize)->f64{ let mut d=vec![];
    for _ in 0..3 { let a1=ms(src,a,5); let b1=ms(src,b,5); let b2=ms(src,b,5); let a2=ms(src,a,5);
        d.push(0.5*(100.0*(b1-a1)/a1+100.0*(b2-a2)/a2)); }
    d.sort_by(|x,y| x.partial_cmp(y).unwrap()); d[1] }
fn main(){
    println!("{:<14}{:>8}{:>8}{:>9}{:>9}{:>9}{:>9}", "corpus","mean ml","null","sz s3","t s3","sz s4","t s4");
    let (mut n3,mut t3,mut n4,mut t4,mut nn,mut c)=(0.0,0.0,0.0,0.0,0.0,0.0);
    let mut rows=vec![];
    for id in IDS {
        let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else {continue};
        let src=&full[..full.len().min(8<<20)];
        rusty_zstd::set_step0_arm(2);
        let _=rusty_zstd::take_rep_rate();
        let s2=rusty_zstd::compress(src,1).unwrap().len() as f64;
        let (_,_,_,allb,alls)=rusty_zstd::take_rep_rate();
        let ml = allb as f64/alls.max(1) as f64;
        rusty_zstd::set_step0_arm(3);
        let z3=rusty_zstd::compress(src,1).unwrap();
        assert_eq!(rusty_zstd::decompress(&z3).unwrap(),src,"{id} s3");
        rusty_zstd::set_step0_arm(4);
        let z4=rusty_zstd::compress(src,1).unwrap();
        assert_eq!(rusty_zstd::decompress(&z4).unwrap(),src,"{id} s4");
        let d3=100.0*(z3.len() as f64-s2)/s2; let d4=100.0*(z4.len() as f64-s2)/s2;
        let null=paired(src,2,2); let p3=paired(src,2,3); let p4=paired(src,2,4);
        println!("{id:<14}{ml:>8.2}{null:>7.2}%{d3:>8.2}%{p3:>8.2}%{d4:>8.2}%{p4:>8.2}%");
        rows.push((*id,ml,d3,d4)); n3+=d3; t3+=p3; n4+=d4; t4+=p4; nn+=null.abs(); c+=1.0;
    }
    println!("\nmean |null| {:.2}%  |  step3: size {:+.2}% time {:+.2}%  |  step4: size {:+.2}% time {:+.2}%",
        nn/c, n3/c, t3/c, n4/c, t4/c);
    rows.sort_by(|a,b| a.1.partial_cmp(&b.1).unwrap());
    println!("\nsorted by mean match length:");
    for (id,ml,d3,d4) in &rows { println!("  {ml:>9.2}  s3 {d3:>7.2}%  s4 {d4:>7.2}%  {id}"); }
    rusty_zstd::set_step0_arm(2);
}
