//! How short WAS the seqs reservation? `last_nseq` was never written by DFast,
//! so seq_guess collapsed to its +64 floor.
const IDS:&[&str]=&["jsonlog-16m","smallmsg-8m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main(){
    println!("{:<14}{:>12}{:>12}{:>14}","corpus","seqs/block","old guess","reallocs saved");
    let mut tot=0u64;
    for id in IDS{
        let Ok(full)=std::fs::read(format!("corpora/data/silesia/{id}"))
            .or_else(|_|std::fs::read(format!("corpora/data/generated/{id}"))) else{continue};
        let src=&full[..full.len().min(2<<20)];
        let _=rusty_zstd::take_dfast_match_stats();
        let _=rusty_zstd::compress(src,3).unwrap();
        let (_mb,sq,_bb,_rb,_rh)=rusty_zstd::take_dfast_match_stats();
        let blocks=((src.len()+131071)/131072) as u64;
        let per=sq/blocks.max(1);
        // doubling from 64: how many growths to reach `per`
        let mut c=64u64; let mut n=0; while c<per {c*=2; n+=1;}
        tot+=n*blocks;
        println!("{id:<14}{per:>12}{:>12}{:>14}",64,n*blocks);
    }
    println!("\nreallocations of `seqs` avoided across the corpus: {tot}");
}
