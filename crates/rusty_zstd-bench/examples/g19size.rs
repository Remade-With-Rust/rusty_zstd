//! The Fast ladder was fitted across 1/2/4/8 MiB (68 cells) BECAUSE samba flipped
//! sign with input size. Re-run that same grid: is it still a win?
const IDS:&[&str]=&["mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray","text-32m","jsonlog-16m","smallmsg-8m","versions-16m","incomp-32m","zeros-32m"];
const CAPS:&[usize]=&[1,2,4,8];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    println!("GATE 5 Fast ladder (L1): ON vs OFF, deterministic SIZE, across input sizes\n");
    println!("{:<14}{:>10}{:>10}{:>10}{:>10}{:>11}","corpus","1 MiB","2 MiB","4 MiB","8 MiB","worst");
    let mut tot=vec![(0i64,0i64);CAPS.len()];
    let mut worst=(0.0f64,String::new());
    for id in IDS{
        let Some(f)=load(id) else{continue};
        let mut row=vec![];
        for (i,mb) in CAPS.iter().enumerate(){
            let src=&f[..f.len().min(mb<<20)];
            if src.len() < (mb<<20) && *mb>1 { row.push(f64::NAN); continue; }
            rusty_zstd::set_g5_fast_arms(2.00, 2.00, 1.0e9);
            let a=rusty_zstd::compress(src,1).unwrap().len() as i64;
            rusty_zstd::set_g5_fast_arms(2.00, 0.70, 2.00);
            let b=rusty_zstd::compress(src,1).unwrap().len() as i64;
            tot[i].0+=a; tot[i].1+=b;
            row.push(100.0*(b-a) as f64/a as f64);
        }
        let w=row.iter().cloned().filter(|x|!x.is_nan()).fold(f64::MIN,f64::max);
        if w>worst.0 {worst=(w,id.to_string());}
        print!("{id:<14}");
        for v in &row{ if v.is_nan(){print!("{:>10}","-");} else {print!("{:>9.3}%",v);} }
        println!("{w:>10.3}%");
    }
    print!("{:<14}","TOTAL");
    for (a,b) in &tot{ print!("{:>9.4}%",100.0*(b-a) as f64/ *a as f64); }
    println!();
    println!("\nworst single cell: {} at {:+.3}%",worst.1,worst.0);
    println!("(the shipped fit claims train -0.1140% / HOLDOUT -0.0766%, worst +0.000%)");
    rusty_zstd::set_g5_fast_arms(2.00, 0.70, 2.00);
}
