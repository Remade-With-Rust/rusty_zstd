//! Is the PAIR search just a slow way of probing every position? There is
//! already a specialised, PIPELINED step-1 loop. Compare, per corpus:
//!   A  step2, pair off   (baseline)
//!   B  step2, pair on    (Gate 6 as shipped)
//!   C  step1, pair off   (the pipelined native every-position loop)
const IDS: &[&str] = &["mr","dickens","webster","ooffice","smallmsg-8m","reymont","osdb","mozilla","samba","xml","nci","sao","x-ray","jsonlog-16m","versions-16m","incomp-32m"];
#[derive(Clone,Copy,PartialEq)] enum A { Base, Pair, Step1 }
fn set(a: A) {
    match a {
        A::Base  => { rusty_zstd::set_step0_arm(2); rusty_zstd::set_pair_on_arm(false); }
        A::Pair  => { rusty_zstd::set_step0_arm(2); rusty_zstd::set_pair_on_arm(true);
                      rusty_zstd::set_pair_gain_arm(0.0); }
        A::Step1 => { rusty_zstd::set_step0_arm(1); rusty_zstd::set_pair_on_arm(false); }
    }
}
fn ms(src:&[u8],a:A,n:usize)->f64{ set(a); let mut b=f64::MAX;
    for _ in 0..n { let t=std::time::Instant::now(); let _=rusty_zstd::compress(src,1).unwrap();
        let e=t.elapsed().as_secs_f64()*1000.0; if e<b{b=e;} } b }
fn sz(src:&[u8],a:A)->f64{ set(a); rusty_zstd::compress(src,1).unwrap().len() as f64 }
fn paired(src:&[u8],a:A,b:A)->f64{ let mut d=vec![];
    for _ in 0..3 { let a1=ms(src,a,5); let b1=ms(src,b,5); let b2=ms(src,b,5); let a2=ms(src,a,5);
        d.push(0.5*(100.0*(b1-a1)/a1+100.0*(b2-a2)/a2)); }
    d.sort_by(|x,y| x.partial_cmp(y).unwrap()); d[1] }
fn main(){
    println!("{:<14}{:>10}{:>10}   {:>10}{:>10}", "corpus","PAIR sz","PAIR t","STEP1 sz","STEP1 t");
    println!("{}","-".repeat(58));
    let (mut ps,mut pt,mut ss,mut st,mut n)=(0.0,0.0,0.0,0.0,0.0);
    for id in IDS {
        let Ok(full)=std::fs::read(format!("corpora/data/silesia/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/generated/{id}"))) else {continue};
        let src=&full[..full.len().min(8*1024*1024)];
        let base=sz(src,A::Base);
        let p=100.0*(sz(src,A::Pair)-base)/base;
        let c=100.0*(sz(src,A::Step1)-base)/base;
        let pm=paired(src,A::Base,A::Pair);
        let cm=paired(src,A::Base,A::Step1);
        println!("{id:<14}{p:>9.2}%{pm:>9.2}%   {c:>9.2}%{cm:>9.2}%");
        ps+=p; pt+=pm; ss+=c; st+=cm; n+=1.0;
        set(A::Step1);
        let z=rusty_zstd::compress(src,1).unwrap();
        assert_eq!(rusty_zstd::decompress(&z).unwrap(),src,"{id} step1 round-trip");
    }
    println!("\nPAIR   size {:+.2}%  time {:+.2}%", ps/n, pt/n);
    println!("STEP1  size {:+.2}%  time {:+.2}%", ss/n, st/n);
}
