//! The 12-pair specialisation set covers (hash_log, chain_log), and BOTH depend
//! on the input SIZE hint. Which (size, level) cells actually hit it?
const PAIRS:&[(u32,u32)]=&[(14,15),(15,15),(17,17),(17,18),(20,20),(21,21),(19,18),(19,19),(22,22),(22,23),(22,24),(23,22),(23,23),(23,24),(24,24)];
fn main(){
    let sizes:&[(usize,&str)]=&[(16<<10,"16K"),(32<<10,"32K"),(64<<10,"64K"),(256<<10,"256K"),(512<<10,"512K"),(1<<20,"1M"),(2<<20,"2M"),(4<<20,"4M"),(8<<20,"8M"),(32<<20,"32M")];
    print!("{:>7}","size");
    for lvl in [13,16,17,18,19,20,21,22]{ print!("{:>10}",format!("L{lvl}")); }
    println!();
    let mut miss=0; let mut tot=0;
    for (n,label) in sizes{
        print!("{label:>7}");
        for lvl in [13i32,16,17,18,19,20,21,22]{
            let p=rusty_zstd::compression_params(lvl,Some(*n as u64)).unwrap();
            let hl=p.hash_log.min(24); let cl=p.chain_log.min(24);
            let hit=PAIRS.contains(&(hl,cl));
            tot+=1; if !hit {miss+=1;}
            print!("{:>10}",format!("{}{},{}",if hit {" "} else {"*"},hl,cl));
        }
        println!();
    }
    println!("\n* = falls through to the RUNTIME body (249 instr, 4 variable shifts)");
    println!("{miss} of {tot} (size, level) cells MISS the specialisation");
}
