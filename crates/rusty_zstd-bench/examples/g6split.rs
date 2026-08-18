//! Does the MAIN probe's slot state predict whether the PAIR probe pays?
//! `m0 == 0` (bucket never written) vs `m0 != 0` (written but rejected).
const IDS: &[&str] = &["mr","dickens","webster","ooffice","smallmsg-8m","reymont","osdb","mozilla","samba","xml","nci","sao","x-ray","jsonlog-16m"];
fn main() {
    rusty_zstd::set_pair_on_arm(true);
    rusty_zstd::set_pair_gain_arm(0.0);
    println!("{:<13} {:>22} {:>22}", "corpus", "m0 EMPTY (never written)", "m0 LIVE (rejected)");
    println!("{:<13} {:>9}{:>7}{:>7} {:>9}{:>7}{:>7}", "", "probes","hit%","B/prb","probes","hit%","B/prb");
    println!("{}","-".repeat(62));
    let (mut pe,mut pl,mut be,mut bl)=(0u64,0u64,0u64,0u64);
    for id in IDS {
        let Ok(full)=std::fs::read(format!("corpora/data/silesia/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/generated/{id}"))) else {continue};
        let src=&full[..full.len().min(8*1024*1024)];
        let _=rusty_zstd::take_pair_split();
        let _=rusty_zstd::compress(src,1).unwrap();
        let (p_e,p_l,h_e,h_l,b_e,b_l)=rusty_zstd::take_pair_split();
        println!("{id:<13} {p_e:>9}{:>6.1}%{:>7.2} {p_l:>9}{:>6.1}%{:>7.2}",
            100.0*h_e as f64/p_e.max(1) as f64, b_e as f64/p_e.max(1) as f64,
            100.0*h_l as f64/p_l.max(1) as f64, b_l as f64/p_l.max(1) as f64);
        pe+=p_e; pl+=p_l; be+=b_e; bl+=b_l;
    }
    println!("\nTOTAL  EMPTY {pe} probes -> {:.3} B/probe", be as f64/pe.max(1) as f64);
    println!("TOTAL  LIVE  {pl} probes -> {:.3} B/probe", bl as f64/pl.max(1) as f64);
    println!("\nEMPTY is {:.0}% of all pair probes", 100.0*pe as f64/(pe+pl).max(1) as f64);
}
