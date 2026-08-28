//! DEFAULT-LEVEL RATIO vs the C reference, which is the claim the README makes.
//! Prints our L3 against a C baseline supplied on stdin as "id bytes" lines.
use std::io::BufRead;
const IDS:&[&str]=&["dickens","webster","samba","mozilla","osdb","mr","nci","xml","ooffice","sao","x-ray","reymont"];
fn main(){
    let mut c=std::collections::HashMap::new();
    for l in std::io::stdin().lock().lines().map_while(Result::ok){
        let mut it=l.split_whitespace();
        if let (Some(k),Some(v))=(it.next(),it.next()){ if let Ok(n)=v.parse::<usize>(){ c.insert(k.to_string(),n); } }
    }
    let (mut ours,mut theirs)=(0usize,0usize);
    println!("{:>10} {:>12} {:>12} {:>10}","corpus","rusty L3","C -3","delta");
    for id in IDS{
        let Ok(f)=std::fs::read(format!("corpora/data/silesia/{id}")).or_else(|_|std::fs::read(format!("corpora/data/generated/{id}"))) else{continue};
        let Some(&cb)=c.get(*id) else{continue};
        let n=rusty_zstd::compress(&f,3).unwrap().len();
        ours+=n; theirs+=cb;
        println!("{:>10} {:>12} {:>12} {:>9.2}%",id,n,cb,100.0*(n as f64-cb as f64)/cb as f64);
    }
    println!("\nTOTAL {:>10} vs {:>10}   default-level gap {:+.2}%",ours,theirs,100.0*(ours as f64-theirs as f64)/theirs as f64);
}
