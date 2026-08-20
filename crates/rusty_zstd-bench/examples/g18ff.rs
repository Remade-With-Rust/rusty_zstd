//! What forfeit does the probe actually measure, and does it separate the
//! corpora where step 2 is free from the ones where it costs?
const IDS:&[&str]=&["sao","mr","dickens","ooffice","samba","mozilla","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    println!("{:<12}{:>13}{:>13}{:>9}{:>14}","corpus","forfeit","seq ratio","probes","true cost");
    let truth=[("sao",-0.420),("mr",-0.225),("dickens",-0.001),("ooffice",1.115),
               ("samba",9.126),("mozilla",13.627),("x-ray",25.053)];
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(8<<20)];
        rusty_zstd::set_step_probe_arm(true);
        let _=rusty_zstd::take_step_forfeit();
        let _=rusty_zstd::compress(src,1).unwrap();
        let (sum,n,sq)=rusty_zstd::take_step_forfeit();
        let t=truth.iter().find(|x|x.0==*id).map(|x|x.1).unwrap_or(0.0);
        if n==0 { println!("{id:<12}{:>13}{:>13}{n:>9}{t:>13.3}%","-","-"); continue; }
        println!("{id:<12}{:>13.4}{:>13.4}{n:>9}{t:>13.3}%", sum as f64/n as f64/10000.0, sq as f64/n as f64/10000.0);
    }
}
