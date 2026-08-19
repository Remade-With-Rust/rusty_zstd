//! GATE 14 @ L19. Step 1: liveness. And FIRST -- is the clock usable here now?
//! 4.45 removed ~100M env lookups from this path; the recorded "+-43% L19 self
//! noise" was largely that contention. Null arm before anything else.
const IDS:&[&str]=&["jsonlog-16m","smallmsg-8m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn ms(src:&[u8],d:usize,lvl:i32,r:usize)->f64{
    rusty_zstd::set_bt_depth_target_arm(d);
    let mut b=f64::MAX;
    for _ in 0..r { let t=std::time::Instant::now(); let _=rusty_zstd::compress(src,lvl).unwrap();
        let e=t.elapsed().as_secs_f64()*1000.0; if e<b{b=e;} }
    b
}
fn paired(src:&[u8],a:usize,b:usize,lvl:i32)->f64{
    let mut d=vec![];
    for _ in 0..3 { let a1=ms(src,a,lvl,3); let b1=ms(src,b,lvl,3);
        let b2=ms(src,b,lvl,3); let a2=ms(src,a,lvl,3);
        d.push(0.5*(100.0*(b1-a1)/a1+100.0*(b2-a2)/a2)); }
    d.sort_by(|x,y|x.partial_cmp(y).unwrap()); d[1]
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(19);
    println!("=== step 1: liveness (does the default differ from the value set?) ===");
    let (mut moved,mut n)=(0,0);
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(1<<20)];
        rusty_zstd::set_bt_depth_target_arm(0);
        let base=rusty_zstd::compress(src,lvl).unwrap();
        let mut any=false;
        for d in [4usize,8,16,64,512]{
            rusty_zstd::set_bt_depth_target_arm(d);
            if rusty_zstd::compress(src,lvl).unwrap()!=base {any=true;}
        }
        n+=1; if any {moved+=1;}
    }
    rusty_zstd::set_bt_depth_target_arm(0);
    println!("corpora whose output moves with the depth target: {moved}/{n}");

    println!("\n=== is the L19 clock usable now? null arm, 1 MiB ===");
    let (mut tn,mut k)=(0.0,0.0);
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(1<<20)];
        let nl=paired(src,0,0,lvl);
        tn+=nl.abs(); k+=1.0;
        print!("{nl:+.1} ");
    }
    println!("\nmean |null| {:.2}%", tn/k);
}
