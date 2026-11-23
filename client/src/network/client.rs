use crossbeam_channel::{Receiver, Sender};
use laminar::{Packet, Socket, SocketEvent};
use std::net::SocketAddr;

#[derive(Clone, Debug)]
pub struct NetworkClient {
    pub sender: Sender<Packet>,
    pub receiver: Receiver<SocketEvent>,
    pub server_addr: SocketAddr,
}

impl NetworkClient {
    pub fn new(server_ip: &str) -> Self {
        // socket (puerto 0 para que la maquina asigne uno libre)
        let mut socket =
            Socket::bind("0.0.0.0:0").expect("No se pudo bindear el socket de cliente");

        let sender = socket.get_packet_sender();
        let receiver = socket.get_event_receiver();
        let server_addr = server_ip.parse().expect("IP de servidor inválida");

        // Lanzamos el hilo de red (polling)
        std::thread::spawn(move || {
            socket.start_polling();
        });

        Self {
            sender,
            receiver,
            server_addr,
        }
    }
}
