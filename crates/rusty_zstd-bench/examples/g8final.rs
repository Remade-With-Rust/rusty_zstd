//! GATE 8 @ L3 -- DETERMINISTIC verdict. The timing instrument's null arm at L3
//! is +-3.71%, larger than the whole effect, so the decision is made on an exact
//! WORK COUNT instead: speculated loads CONSUMED (benefit, zero added work) vs
//! DISCARDED (pure added work).
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
#[derive(Clone,Copy,PartialEq)] enum A { Off, Const, Disp }
fn set(a:A){ match a {
    A::Off   => rusty_zstd::set_dfast_pipe_arm(false),
    A::Const => { rusty_zstd::set_dfast_pipe_arm(true); rusty_zstd::set_dfast_spec_min_arm(0.0); }
    A::Disp  => { rusty_zstd::set_dfast_pipe_arm(true); rusty_zstd::set_dfast_spec_min_arm(0.70); }
}}
fn run(src:&[u8],a:A)->(Vec<u8>,u64,u64){
    set(a);
    let _=rusty_zstd::take_dfast_spec();
    let z=rusty_zstd::compress(src,3).unwrap();
    let (made,used)=rusty_zstd::take_dfast_spec();
    (z,made,used)
}
fn main(){
    println!("{:<14}{:>12}{:>10}   {:>12}{:>10}{:>9}", "corpus","CONST made","wasted","DISP made","wasted","kept use");
    println!("{}","-".repeat(70));
    let (mut cm,mut cw,mut dm,mut dw,mut cu,mut du)=(0u64,0u64,0u64,0u64,0u64,0u64);
    let mut bad=0;
    for id in IDS {
        let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else {continue};
        let src=&full[..full.len().min(8<<20)];
        let (z0,_,_)=run(src,A::Off);
        let (z1,m1,u1)=run(src,A::Const);
        let (z2,m2,u2)=run(src,A::Disp);
        if z0!=z1 || z0!=z2 { bad+=1; println!("{id:<14} BYTE DIVERGENCE"); }
        assert_eq!(rusty_zstd::decompress(&z2).unwrap(),src,"{id} round-trip");
        println!("{id:<14}{m1:>12}{:>9.1}%   {m2:>12}{:>9.1}%{:>8.0}%",
            100.0*(m1-u1) as f64/m1.max(1) as f64,
            100.0*(m2-u2) as f64/m2.max(1) as f64,
            100.0*u2 as f64/u1.max(1) as f64);
        cm+=m1; cw+=m1-u1; cu+=u1; dm+=m2; dw+=m2-u2; du+=u2;
    }
    println!("\nbyte divergences: {bad}/18   (must be 0 -- this is an issue-order change)");
    println!("CONSTANT ON : {cm} speculated, {cw} WASTED ({:.1}%)", 100.0*cw as f64/cm.max(1) as f64);
    println!("DISPATCHED  : {dm} speculated, {dw} WASTED ({:.1}%)", 100.0*dw as f64/dm.max(1) as f64);
    println!("\n-> wasted loads cut {:.1}%, overlap benefit retained {:.1}%",
        100.0*(cw-dw) as f64/cw.max(1) as f64, 100.0*du as f64/cu.max(1) as f64);
    set(A::Disp);
}
