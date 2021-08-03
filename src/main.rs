
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use std::sync::Arc;
use std::time::Duration;
use rustop::opts;

mod rfb_session;
mod screen;
mod locator;
mod query;
mod resources;

use screen::Screen;

pub type ScreenLock = Arc<Mutex<Screen>>;

#[derive(Debug, Clone, Copy)]
enum SessionState {
    LocateServersManager,
    ConnectToServer,
    QueryServersManager,
    RfbSession,
}

struct StateManager {
    screen: ScreenLock,
    query_bytes: Vec<u8>,

    servers_manager: Option<String>,
    server_address: Option<String>,
    stream: Option<TcpStream>,
}

impl StateManager {
    fn new(name: &str) -> StateManager {
        let screen = Screen::new().expect("Error while creating screen object");
        let query_bytes = query::prepare_query(name, &screen);

        StateManager {
            screen: Arc::new(Mutex::new(screen)),
            query_bytes,
            servers_manager: None,
            server_address: None,
            stream: None,
        }
    }

    async fn connect_to_server(server_address: &str) -> Option<TcpStream> {
        let timeout = tokio::time::sleep(Duration::from_secs(3));
        tokio::pin!(timeout);
    
        tokio::select! {
            result = TcpStream::connect(server_address) => {
                match result {
                    Ok(stream) => Some(stream),
                    Err(_) => None,
                }
            },
            _ = &mut timeout => None
        }
    }

    async fn do_domain_session(&mut self, domain_name: &str) {
        let mut state: SessionState = SessionState::LocateServersManager;

        loop {
            match state {
                SessionState::LocateServersManager => {
                    {
                        let mut screen = self.screen.lock().await;
                        
                        screen.display_png_resource(resources::LOOKING_FOR_MANAGER_IMAGE);
                    }

                    loop {
                        if let Ok(Some(servers_manager)) = locator::locate_ht_manager(domain_name).await {
                            self.servers_manager = Some(servers_manager);
                            state = SessionState::QueryServersManager;
                            break;
                        }
                        println!("Could not locate domain '{}'", domain_name);
                    };
                },

                SessionState::QueryServersManager => {
                    {
                        let mut screen = self.screen.lock().await;
                        
                        screen.display_png_resource(resources::QUERY_FOR_SERVER_IMAGE);
                    }

                    match query::query_for_hometouch_server(self.servers_manager.as_ref().unwrap(), &self.query_bytes).await {
                        Some(server_address) => {
                            self.server_address = Some(server_address);
                            state = SessionState::ConnectToServer;
                        },
                        None => {
                            self.servers_manager = None;
                            state = SessionState::LocateServersManager;
                        }
                    };
                },

                SessionState::ConnectToServer => {
                    {
                        let mut screen = self.screen.lock().await;
                        
                        screen.display_png_resource(resources::CONNECTING_TO_SERVER_IMAGE);
                    }

                    match Self::connect_to_server(&self.server_address.as_ref().unwrap()).await {
                        Some(stream) => {
                            self.stream = Some(stream);
                            state = SessionState::RfbSession;
                        },
                        None => {
                            self.server_address = None;
                            state = SessionState::QueryServersManager;
                        },
                    };
                },

                SessionState::RfbSession => {
                    println!("{} managed by {} -> {}", domain_name, self.servers_manager.as_ref().unwrap(), self.server_address.as_ref().unwrap());
                    let _ = rfb_session::run(self.stream.take().unwrap(), self.screen.clone()).await;
                    state = SessionState::ConnectToServer;
                },
            }
        }
    }

    async fn do_server_session(&mut self, server_address: &str) {
        let mut state = SessionState::ConnectToServer;

        loop {
            match state {
                SessionState::ConnectToServer => {
                    {
                        let mut screen = self.screen.lock().await;
                        
                        screen.display_png_resource(resources::CONNECTING_TO_SERVER_IMAGE);
                    }

                    match Self::connect_to_server(server_address).await {
                        Some(stream) => {
                            self.stream = Some(stream);
                            state = SessionState::RfbSession;
                        },
                        None => {
                            println!("Connection to {} failed, retry in 3 seconds", server_address);
                            tokio::time::sleep(Duration::from_secs(3)).await;
                        }
                    }
                }
                SessionState::RfbSession => {
                    let _ = rfb_session::run(self.stream.take().unwrap(), self.screen.clone()).await;
                    state = SessionState::ConnectToServer;
                },
                s => panic!("Unexpected state: {:?}", s),
            }
        }
    }
}

#[tokio::main]
async fn main() {
    let (args, _) = opts! {
        synopsis "Hometouch server client";
        opt server:Option<String>, desc: "Connect to specific HomeTouch (RFB) server";
        opt name:String = gethostname::gethostname().into_string().unwrap();
        opt domains:bool=false, desc: "List available Hometoucher domains (_HtVncConf._udp.local)";
        param domain:Option<String>, desc: "Domain to connect to (e.g 'Beit Zait House' or 'Tel-Aviv Apt')";
    }.parse_or_exit();

    if args.domains {
        match locator::get_domains_list().await {
            Ok(domains) => {
                println!("Found {} domains:", domains.len());
                for (name, address) in domains.iter() {
                    println!("{} -> {}", name, address);
                }
            },
            Err(e) => eprintln!("Error obtaining Hometoucher domains: {}", e),
        }

        std::process::exit(0);
    }

    if let Ok(_) = Screen::set_console_to_graphic_mode() {
        ctrlc::set_handler(move || {
            let _ = Screen::set_console_to_text_mode();
            std::process::exit(0);
        }).expect("Failed to set ctrl-c handler");
    }
    else {
        eprintln!("Failed to set /dev/console to graphics mode (run with sudo or as service)")
    }

    let mut state_manager = StateManager::new(&args.name);

    if let Some(domain) = args.domain {
        state_manager.do_domain_session(&domain).await;
    }
    else if let Some(server) = args.server {
        state_manager.do_server_session(&server).await;
    }
    else {
        eprintln!("Either --server <server> or <domain name> must be specified");
    }
}
