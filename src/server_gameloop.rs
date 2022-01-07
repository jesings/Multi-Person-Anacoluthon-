use std::sync::{*, mpsc::*};
use std::collections::VecDeque;

use crate::gamestate::{Gamedata, GDTuple};
use crate::net::{pkt::PktPayload, *};
use crate::player::player::*;
use crate::map::grid::*;

pub fn serveloop((mut stream, addr): (std::net::TcpStream, std::net::SocketAddr), gd: Arc<Gamedata>, sender: mpsc::Sender<PktPayload>, mut br: bus::BusReader<Arc<PktPayload>>, livelisteners: Arc<atomic::AtomicUsize>, index: usize) -> Result<(), String> {

    pkt::send_pkt(&mut stream, Arc::new(PktPayload::Gamedata(GDTuple {0: gd.players.iter().map(|x| (*x.lock().unwrap()).clone()).collect(), 1: 0i128, 2: index}))).expect("Could not send initialization packet");

    loop {
        let mut killflag = false;
        loop {
            match pkt::recv_pkt(&mut stream)  {
                Ok(recvd) => {
                    sender.send(recvd).unwrap();
                }
                Err(s) => {
                    if s.as_str() == "Fatal" {
                        livelisteners.fetch_sub(1, atomic::Ordering::Relaxed);
                        killflag = true;
                    }
                    break;
                }
            }
        }
        if killflag || livelisteners.load(atomic::Ordering::Relaxed) == 0 {
            break;
        }
        //if this doesn't run, assume for now it's just because we're nonblocking

        while let Ok(recvd) = br.try_recv() {
            if let Err(s) = pkt::send_pkt(&mut stream, recvd) {
                if s.as_str() == "Fatal" {
                    livelisteners.fetch_sub(1, atomic::Ordering::Relaxed);
                    killflag = true;
                }
            }
        }

        if killflag || livelisteners.load(atomic::Ordering::Relaxed) == 0 {
            break;
        }

        std::thread::sleep(std::time::Duration::new(0, 1_000_000_000u32 / 1000));
    }

    return Ok(());
}

pub fn gameloop() {
    let streams = servnet::initialize_server("127.0.0.1:9495".to_string());
    let mut spmc = bus::Bus::new(2048);

    let livelisteners = Arc::new(atomic::AtomicUsize::new(streams.len()));

    let (mpsc_tx, mpsc_rx) = channel();

    let seed = rand::random::<i128>();
    let mut playarrs = vec!();
    for i in 0..streams.len() {
        playarrs.push(Player::test_player(i));
    }

    let gd = Arc::new(Gamedata {
        players: playarrs.drain(..).map(|x| Arc::new(Mutex::new(x))).collect(),
        grid: Grid::gen_blank_grid(640, 480),
    });
    let handles = servnet::launch_server_workers(streams, gd.clone(), mpsc_tx, &mut spmc, livelisteners.clone());

    //figure out how to kill gracefully
    let mut broadcasts_needed = VecDeque::new();
    loop {
        while let Ok(recvd) = mpsc_rx.try_recv() {
            if let PktPayload::Delta(ref deltalist) = recvd {
                for delta in deltalist {
                    let mut deltaplayer = gd.players[delta.pid].lock().unwrap();
                    deltaplayer.pos.0 += delta.poschange.0;
                    deltaplayer.pos.1 += delta.poschange.1;
                    //check that this position is valid, if not revert!?
                }
            }
            broadcasts_needed.push_back(recvd);
        }

        while broadcasts_needed.len() > 0 {
            let frontel = broadcasts_needed.pop_front().unwrap();
            let tryfront = Arc::new(frontel);
            if let Err(_) = spmc.try_broadcast(tryfront.clone()) {
                break;
            } else {
                broadcasts_needed.push_front(Arc::try_unwrap(tryfront).unwrap());
            }
        }

        if livelisteners.load(atomic::Ordering::Relaxed) == 0 {
            break;
        }

        std::thread::sleep(std::time::Duration::new(0, 1_000_000_000u32 / 1000));
    }
    for handle in handles {
        handle.join().unwrap();
    }
}
