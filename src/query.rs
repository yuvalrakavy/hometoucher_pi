
use std::collections::HashMap;
use std::time::Duration;
use tokio::net::UdpSocket;
use super::screen::Screen;

pub fn prepare_query(my_name: &str, screen: &Screen) -> Vec<u8> {
    let query = std::array::IntoIter::new(
        [
            ("Name", String::from(my_name)),
            ("ScreenWidth", screen.xres().to_string()),
            ("ScreenHeight", screen.yres().to_string()),
            ("FormFactor", String::from("InWallPanel")),
        ]
    ).collect();

    get_query_bytes(&query)
}

async fn do_query_for_hometouch_server(servers_manager_address: &str, query_bytes: &[u8], timeout: Duration) -> Option<String> {
    let socket = UdpSocket::bind("0.0.0.0:0").await.expect("Query socket binding failed");
    let mut reply_bytes: Vec<u8> = vec![0; 1024];

    socket.send_to(query_bytes, servers_manager_address).await.expect("Query send failed");

    let timeout = tokio::time::sleep(timeout);
    tokio::pin!(timeout);

    tokio::select! {
        Ok(_) = socket.recv_from(&mut reply_bytes[..]) => {
            let reply = parse_query_bytes(&reply_bytes);
            Some(extact_server_address(&reply))
        },
        _ = &mut timeout => None
    }
}

pub async fn query_for_hometouch_server(servers_manager_address: &str, query_bytes: &[u8]) -> Option<String> {
    for _ in 0..3 {
        let result = do_query_for_hometouch_server(servers_manager_address, query_bytes, Duration::from_secs(3)).await;

        if result.is_some() {
            return result;
        }
    }

    None
}

fn get_query_bytes(query: &HashMap<&str, String>) -> Vec<u8> {
    let mut query_bytes = Vec::<u8>::new();
    query.iter().for_each(|(k, v)| {
        add_value(k, &mut query_bytes);
        add_value(v, &mut query_bytes);
    });

    add_value("", &mut query_bytes);
    add_value("", &mut query_bytes);

    query_bytes
}

fn add_value(value: &str, query_bytes: &mut Vec<u8>) {
    let byte_count = value.bytes().len();

    query_bytes.push((byte_count >> 8) as u8);
    query_bytes.push((byte_count & 0xFF) as u8);

    query_bytes.extend_from_slice(value.as_bytes());
}

fn parse_query_bytes(query_bytes: &[u8]) -> HashMap<String, String> {
    let mut result = HashMap::<String, String>::new();
    let mut i = 0;
    let mut get_value = || -> (usize, String) {
        let count = ((query_bytes[i] as usize) << 8) + query_bytes[i + 1] as usize;
        i += 2;
        let result = String::from_utf8(query_bytes[i..i+count].to_vec()).expect("Invalid query result value");

        i += count;
        (count, result)
    };

    loop {
        let (count, name) = get_value();
        if count == 0 {
            break;
        } else {
            let(_, value) = get_value();
            result.insert(name, value);
        }
    }

    result
}

fn extact_server_address(query_result: &HashMap<String, String>) -> String {
    let server = query_result.get("Server").expect("Server not found in query result");
    let port = query_result.get("Port").expect("Port not found in query result");

    format!("{}:{}", server, port)
}
