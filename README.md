# pfr-api

Scraper for pro-football-reference.com player stats. This tool can currently download, parse, and write CSVs of game-level data.

# Usage
The following invocation will write CSV files with 2000 data to the `output` directory.

```pfr-api --year 2000 --output-dir output```

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
Each file is a CSV corresponding to a player stats table in a pro-football-reference game log page (e.g. like this [one](https://www.pro-football-reference.com/boxscores/202109120buf.htm)).

If you want to download multiple years at once, you can use this snippet:

```echo {2015..2020} | xargs -n 1 pfr-api --output-dir output --year```

Just be careful of potentially getting rate limited :)