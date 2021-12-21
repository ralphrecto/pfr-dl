use hyper::{
    http::Uri,
    body::{to_bytes},
    Client
};
use hyper_tls::HttpsConnector;
use std::{error::Error, collections::HashMap};
use scraper::{Html, Selector, Node, ElementRef};
use regex::Regex;
use lazy_static::lazy_static;

lazy_static!{
    static ref GAME_ID_REGEX: Regex = Regex::new(r".*/(\w+)\.htm").unwrap();
}

#[derive(Debug)]
struct PlayerGameStats<'a> {
    player_id: &'a str,
    player_name: &'a str,
    stats: HashMap<&'a str, &'a str>
}

#[derive(Debug)]
struct GameStats<'a> {
    game_id: &'a str,
    player_stats: Vec<PlayerGameStats<'a>>
}

fn selector(selector_str: &str) -> Selector {
    Selector::parse(selector_str).unwrap()
}

fn parse_player_stats_table<'a>(html_doc: &'a Html, table_selector_str: &str) -> Vec<PlayerGameStats<'a>> {
    let table_selector = selector(table_selector_str);
    let table_elt= html_doc.select(&table_selector).next().unwrap();

    let tbody_selector= selector("tbody");
    let table_body = table_elt.select(&tbody_selector).next().unwrap();

    let mut retlist: Vec<PlayerGameStats> = vec![];
    'rowlabel: for table_row in table_body.children() {

        let mut player_data: HashMap<&str, &str> = HashMap::new();

        for table_data_noderef in table_row.children() {
            let table_data_elt = match table_data_noderef.value() {
                Node::Element(elt) => elt,
                _ => continue
            };

            // Handle player id.

            match table_data_elt.attr("data-append-csv") {
                Some(id) => {
                    player_data.insert("player_id", id);
                },
                None => ()
            };

            // Handle stat.

            let stat_name = match table_data_elt.attr("data-stat") {
                Some(s) => s,
                None => {
                    // This indicates the table body row is not a data row.
                    continue 'rowlabel
                }
            };

            let data_val_elt = if stat_name == "player" {
                table_data_noderef
                    .first_child()
                    .and_then(|n| n.first_child())
                    .map(|n| n.value())
            } else {
                table_data_noderef
                    .first_child()
                    .map(|v| v.value())
            };

            match data_val_elt {
                Some(Node::Text(t)) => {
                    player_data.insert(stat_name, t.text.as_ref());
                },
                _ => ()
            };
        }

        match (player_data.get("player_id"), player_data.get("player")) {
            (Some(&player_id), Some(&player_name)) => {
                retlist.push(PlayerGameStats {
                    player_id,
                    player_name,
                    stats: player_data
                });
            },
            _ => ()
        }
    }

    retlist
}

fn parse_game_log<'a>(game_log_html: &'a Html) -> GameStats<'a> {
    let mut player_stats: Vec<PlayerGameStats> = vec![];

    let game_link_selector = selector("link[rel=canonical]");
    let game_link = game_log_html.select(&game_link_selector).next().unwrap().value().attr("href").unwrap();
    let game_id = GAME_ID_REGEX.captures(game_link).unwrap().get(1).unwrap().as_str();

    player_stats.extend(parse_player_stats_table(game_log_html, "#player_offense"));
    player_stats.extend(parse_player_stats_table(game_log_html, "#player_defense"));
    player_stats.extend(parse_player_stats_table(game_log_html, "#returns"));
    player_stats.extend(parse_player_stats_table(game_log_html, "#kicking"));
    player_stats.extend(parse_player_stats_table(game_log_html, "#passing_advanced"));
    player_stats.extend(parse_player_stats_table(game_log_html, "#rushing_advanced"));
    player_stats.extend(parse_player_stats_table(game_log_html, "#receiving_advanced"));
    player_stats.extend(parse_player_stats_table(game_log_html, "#defense_advanced"));

    GameStats { game_id, player_stats }
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
    let game_log_html_str: String = bytes.iter().map(|&c| c as char).collect();
    // TODO a lot of the stats tables below are commented!
    // find a better way to uncomment them
    let s_prime = game_log_html_str.replace("\n<!--", "").replace("\n-->", "");
    let html_doc = Html::parse_document(&s_prime);

    let game_stats = parse_game_log(&html_doc);
    println!("{:?}", game_stats);

    Ok(())
}