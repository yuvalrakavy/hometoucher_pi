
use std::{time::Duration};

const HT_MANAGER_SERVICE: &'static str = "_HtVncConf._udp.local";
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(5);

pub async fn locate_ht_manager(domain_name: &str) -> Result<Option<String>, mdns::Error> {
    let mut host_name = domain_name.to_owned();
    
    host_name.push_str(".");
    host_name.push_str(HT_MANAGER_SERVICE);

    let result = mdns::resolve::one(HT_MANAGER_SERVICE, host_name, RESOLVE_TIMEOUT).await?;

    match result {
        Some(response) => {
            let mut result = get_server_name(&response);

            result.push_str(":");
            result.push_str(&get_port(&response));

            Ok(Some(result))
        },
        None => Ok(None)
    }
}

fn get_server_name(response: &mdns::Response) -> String {
    let addr = response.records().find_map(
        |record| match record.kind {
            mdns::RecordKind::A(addr) => Some(addr.to_string()),
            mdns::RecordKind::AAAA(addr) => Some(addr.to_string()),
            _ => None
        });

    addr.expect(&format!("Cannot extract address from mdns response: {:#?}", response))
}

fn get_port(response: &mdns::Response) -> String {
    let port = response.records().find_map(
        |record| match record.kind {
            mdns::RecordKind::SRV{port, ..} => Some(port.to_string()),
            _ => None
        });

    port.expect(&format!("Cannot extract port from mdns response: {:#?}", response))
}
