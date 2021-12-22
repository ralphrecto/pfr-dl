# pfr-dl

Scraper for pro-football-reference.com player stats. This tool can currently download, parse, and write CSVs of game-level data and historical player lists.

# Usage
## Game data
The following invocation will write CSV files with 2000 data to the `output` directory.

```pfr-dl --year 2000 --output-dir output```

After invocation, `output` will look like the following on the file system:

```
$ tree output
output
├── 2000
│   ├── 1
│   │   ├── Defense
│   │   ├── Kicking
│   │   ├── Offense
│   │   └── Returns
│   ├── 10
│   │   ├── Defense
│   │   ├── Kicking
│   │   ├── Offense
│   │   └── Returns
...
```
Within the output directory, there will be a directory for the year. Within a year directory, there will be 1 directory per regular season week for that year. Within each week directory will be a collection of CSV files with all of the stats for that week's games for each dimension of the game (Offense, Defense, etc.) Each CSV file corresponds to a player stats table in a pro-football-reference game log page (e.g. like this [one](https://www.pro-football-reference.com/boxscores/202109120buf.htm)).

If you want to download multiple years at once, you can use this snippet:

```echo {2015..2020} | xargs -n 1 pfr-dl --output-dir output --year```

Just be careful of potentially getting rate limited :)

## Historical player data
The following command will download a list of all historical players (including their positions and active years) to a `players` file in the output dir.

```pfr-dl --mode player```