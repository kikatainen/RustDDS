use mio::{Poll, Event, Events, Token, Ready, PollOpt};
use mio_extras::channel as mio_channel;

use std::collections::HashMap;

use crate::dds::message_receiver::MessageReceiver;
use crate::dds::reader::Reader;
use crate::network::udp_listener::UDPListener;
use crate::network::constant::*;
use crate::structure::guid::{GuidPrefix};

pub struct DPEventWrapper {
  poll: Poll,
  udp_listeners: HashMap<Token, UDPListener>,
  send_targets: HashMap<Token, mio_channel::Sender<Vec<u8>>>,
  message_receiver: MessageReceiver,
  receivers: HashMap<Token, mio_extras::channel::Receiver<i32>>,
}

impl DPEventWrapper {
  pub fn new(
    udp_listeners: HashMap<Token, UDPListener>,
    send_targets: HashMap<Token, mio_channel::Sender<Vec<u8>>>,
    participant_guid_prefix: GuidPrefix,
  ) -> DPEventWrapper {
    let poll = Poll::new().expect("Unable to create new poll.");

    let mut udp_listeners = udp_listeners;

    for (token, listener) in &mut udp_listeners {
      poll
        .register(
          listener.mio_socket(),
          token.clone(),
          Ready::readable(),
          PollOpt::edge(),
        )
        .expect("Failed to register listener.");
    }

    DPEventWrapper {
      poll: poll,
      udp_listeners: udp_listeners,
      send_targets: send_targets,
      message_receiver: MessageReceiver::new(participant_guid_prefix),
      receivers: HashMap::new(),

    }
  }

  pub fn event_loop(mut ev_wrapper: DPEventWrapper){
    loop {
      let mut events = Events::with_capacity(1024);

      ev_wrapper.poll.poll(
        &mut events, None
      ).expect("Failed in waiting of poll.");

      println!("Number of events: {:?}", events.len());

      for event in events.into_iter() {
        println!("{:?}", event);

        if event.token() == STOP_POLL_TOKEN {
          return;
        } else if DPEventWrapper::is_udp_traffic(&event) {
          ev_wrapper.handle_udp_traffic(&event);
        } else if DPEventWrapper::is_reader_action(&event) {
          ev_wrapper.handle_reader_action(&event);
        }
          //ev_wrapper.message_receiver.remove_reader(Reader::new());
        }
      }
    }

  pub fn is_udp_traffic(event: &Event) -> bool {
    event.token() == DISCOVERY_SENDER_TOKEN || event.token() == USER_TRAFFIC_SENDER_TOKEN
  }

  pub fn is_reader_action(event: &Event) -> bool {
    event.token() == ADD_READER_TOKEN || event.token() == REMOVE_READER_TOKEN
  }

  pub fn handle_udp_traffic(&mut self, event: &Event) {
    let listener = self.udp_listeners.get(&event.token());
      match listener {
        Some(l) => loop {
          let data = l.get_message();
          if data.is_empty() {
            return;
          }

          if event.token() == DISCOVERY_SENDER_TOKEN {
            self.message_receiver.handle_discovery_msg(data);
          } else if event.token() == USER_TRAFFIC_SENDER_TOKEN {
            self.message_receiver.handle_user_msg(data);
          }
        },
        None => return,
      };
  }

  pub fn handle_reader_action(&mut self, event: &Event) {
    if let Some(rec) = self.receivers.get(&event.token()){
      let data = rec.try_recv().unwrap();
      println!("Received data: {}", data);
      match event.token() {
        ADD_READER_TOKEN =>{
          //self.message_receiver.add_reader(Reader::new());
        },
        REMOVE_READER_TOKEN =>{
          //self.message_receiver.remove_reader(Reader::new());
        },
        _ =>{},
      }
    }
  }
}


#[cfg(test)]
mod tests {
  use super::*;
  use std::thread;
  use std::time::Duration;
  //use std::sync::mpsc;
  
  #[test]
  fn dpew_add_readers() {

    let mut dp_event_wrapper = DPEventWrapper::new(
      HashMap::new(),
      HashMap::new(),
      GuidPrefix::default(),
    );

    let (sender_add, receiver) = mio_channel::channel::<i32>();
    let (sender_remove, receiver1) = mio_channel::channel::<i32>();
    let (sender_stop, receiver2) = mio_channel::channel::<i32>();

    dp_event_wrapper.receivers.insert(ADD_READER_TOKEN, receiver);
    dp_event_wrapper.receivers.insert(REMOVE_READER_TOKEN, receiver1);
    dp_event_wrapper.receivers.insert(STOP_POLL_TOKEN, receiver2);

    for (token, listener) in &mut dp_event_wrapper.receivers {
      dp_event_wrapper.poll
        .register(
          listener,
          token.clone(),
          Ready::readable(),
          PollOpt::edge(),
        )
        .expect("Failed to register listener.");
    }

    let child = thread::spawn(
      move || DPEventWrapper::event_loop(dp_event_wrapper)
    );
    
    let n = 3;
    for i in 0..n {
      println!("Sent data {}", i);
      sender_add.send(i).unwrap();
      std::thread::sleep(Duration::new(0,50));
    }
    println!("poistetaan eka");
    sender_remove.send(-1).unwrap();
    std::thread::sleep(Duration::new(0,50));
    println!("poistetaan toinen");
    sender_remove.send(-2).unwrap();
    println!("Lopetustoken lähtee");
    sender_stop.send(0).unwrap();
    child.join().unwrap();
  }
}
