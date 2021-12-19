use hyper::{
    http::Uri,
    body::{to_bytes},
    Client
};
use hyper_tls::HttpsConnector;
use std::error::Error;
use scraper::{Html, Selector, Node, ElementRef};

fn parse_player_stats_table(table_elt: &ElementRef) {
    let tbody_selector= Selector::parse("tbody").unwrap();
    let table_body = table_elt.select(&tbody_selector).next().unwrap();

    'rowlabel: for data_row in table_body.children() {

        for data_elt in data_row.children() {
            let stat_name_opt = match data_elt.value() {
                Node::Element(elt) => elt.attr("data-stat"),
                _ => None
            };

            let stat_name = match stat_name_opt {
                Some(s) => s,
                None => {
                    // This indicates the table body row is not a data row.
                    continue 'rowlabel
                }
            };

            let data_val_elt = if stat_name == "player" {
                data_elt
                .first_child()
                .and_then(|n| n.first_child())
                .map(|n| n.value())
            } else {
                data_elt
                .first_child()
                .map(|v| v.value())
            };

            let data_val: Option<String> = match data_val_elt {
                // TODO get rid of allocation via format here
                Some(Node::Text(t)) => Some(format!("{}", t.text)),
                _ => None
            };

            println!("stat {}, val {:?}", stat_name, data_val);
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let https = HttpsConnector::new();
    let client = Client::builder().build::<_, hyper::Body>(https);
    let uri: Uri = "https://www.pro-football-reference.com/boxscores/202111280nyg.htm".parse()?;

    let mut resp = client.get(uri).await?;
    // println!("Resp status: {}", resp.status());

    let mut body = resp.into_body();
    // NOTE response header says it is UTF-8 but UTF-8 parsing fails...
    // `file` claims the bytes are actually ISO-8859-1.
    // TODO find a valid way to convert to text
    let bytes = to_bytes(body).await.unwrap();
    let s: String = bytes.iter().map(|&c| c as char).collect();

    // TODO a lot of the stats tables below are commented!
    // find a better way to uncomment them
    let s_prime = s.replace("\n<!--", "").replace("\n-->", "");
    let html_doc = Html::parse_document(&s_prime);

    let offense_selector = Selector::parse("#player_offense").unwrap();
    let defense_selector = Selector::parse("#player_defense").unwrap();
    let returns_selector = Selector::parse("#returns").unwrap();
    let kicking_selector = Selector::parse("#kicking").unwrap();
    let passing_adv_selector = Selector::parse("#passing_advanced").unwrap();
    let rushing_adv_selector = Selector::parse("#rushing_advanced").unwrap();
    let receiving_adv_selector = Selector::parse("#receiving_advanced").unwrap();
    let defense_adv_selector = Selector::parse("#defense_advanced").unwrap();

    let offense_table = html_doc.select(&offense_selector).next().unwrap();
    let defense_table = html_doc.select(&defense_selector).next().unwrap();
    let returns_table = html_doc.select(&returns_selector).next().unwrap();
    let kicking_table = html_doc.select(&kicking_selector).next().unwrap();
    let passing_adv_table = html_doc.select(&passing_adv_selector).next().unwrap();
    let rushing_adv_table = html_doc.select(&rushing_adv_selector).next().unwrap();
    let receiving_adv_table = html_doc.select(&receiving_adv_selector).next().unwrap();
    let defense_adv_table = html_doc.select(&defense_adv_selector).next().unwrap();

    parse_player_stats_table(&offense_table);
    parse_player_stats_table(&defense_table);
    parse_player_stats_table(&returns_table);
    parse_player_stats_table(&kicking_table);
    parse_player_stats_table(&passing_adv_table);
    parse_player_stats_table(&rushing_adv_table);
    parse_player_stats_table(&receiving_adv_table);
    parse_player_stats_table(&defense_adv_table);

    Ok(())
}