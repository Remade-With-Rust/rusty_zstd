//! GATE 6 FINAL — the three-way route vs the two constants.
//!   A  OFF     pair never runs           (the pre-Gate-6 encoder)
//!   B  PAIR    pair always runs          (what shipped)
//!   C  ROUTE   off / step-1 / pair by measured bytes-per-probe
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
#[derive(Clone,Copy,PartialEq)] enum A { Off, Pair, Route }
fn set(a:A){ match a {
    A::Off   => { rusty_zstd::set_pair_on_arm(false); }
    A::Pair  => { rusty_zstd::set_pair_on_arm(true); rusty_zstd::set_pair_gain_arm(0.0);
                  rusty_zstd::set_pair_hi_arm(0.0); }   // rate>=0 -> always route 2
    A::Route => { rusty_zstd::set_pair_on_arm(true); rusty_zstd::set_pair_gain_arm(0.20);
                  rusty_zstd::set_pair_hi_arm(std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3.0)); }
}}
fn ms(src:&[u8],a:A,n:usize)->f64{ set(a); let mut b=f64::MAX;
    for _ in 0..n { let t=std::time::Instant::now(); let _=rusty_zstd::compress(src,1).unwrap();
        let e=t.elapsed().as_secs_f64()*1000.0; if e<b{b=e;} } b }
fn sz(src:&[u8],a:A)->f64{ set(a); rusty_zstd::compress(src,1).unwrap().len() as f64 }
fn paired(src:&[u8],a:A,b:A)->f64{ let mut d=vec![];
    for _ in 0..3 { let a1=ms(src,a,5); let b1=ms(src,b,5); let b2=ms(src,b,5); let a2=ms(src,a,5);
        d.push(0.5*(100.0*(b1-a1)/a1+100.0*(b2-a2)/a2)); }
    d.sort_by(|x,y| x.partial_cmp(y).unwrap()); d[1] }
fn main(){
    println!("vs pair OFF.   B = always pair (shipped)   C = routed\n");
    println!("{:<14}{:>10}{:>10}   {:>10}{:>10}","corpus","B size","B time","C size","C time");
    println!("{}","-".repeat(58));
    let (mut bs,mut bt,mut cs,mut ct,mut n)=(0.0,0.0,0.0,0.0,0.0);
    for id in IDS {
        let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else {continue};
        let src=&full[..full.len().min(8*1024*1024)];
        let base=sz(src,A::Off);
        let b=100.0*(sz(src,A::Pair)-base)/base;
        let c=100.0*(sz(src,A::Route)-base)/base;
        let bm=paired(src,A::Off,A::Pair);
        let cm=paired(src,A::Off,A::Route);
        println!("{id:<14}{b:>9.2}%{bm:>9.2}%   {c:>9.2}%{cm:>9.2}%");
        bs+=b; bt+=bm; cs+=c; ct+=cm; n+=1.0;
        set(A::Route);
        let z=rusty_zstd::compress(src,1).unwrap();
        assert_eq!(rusty_zstd::decompress(&z).unwrap(),src,"{id} round-trip");
    }
    println!("\nB always-pair  size {:+.2}%  time {:+.2}%", bs/n, bt/n);
    println!("C routed       size {:+.2}%  time {:+.2}%", cs/n, ct/n);
}
