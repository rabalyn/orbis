# Caching and tile servers

Goal: browsing should put as little load as possible on the public services that provide
the data. No tile is fetched twice if that can be avoided, and a struggling server is left
alone. Where better imagery exists, it is used as the view gets closer to the ground.

## How a tile is found

Each step is tried only if the one before has no answer.

1. **Local disk cache**, including the memory of what a server does not have.
2. **A download already in progress**: whoever needs the same tile waits for that result.
3. **LAN cache servers**, if any are configured or discovered.
4. **The public server**, unless it is timed out.

## Server tracking

Every public server, identified by its host name, is tracked separately.

| Event | Reaction |
|---|---|
| 3 server errors (HTTP 5xx) or request timeouts within 60 s | **Whole server timed out for 15 minutes** |
| HTTP 429 (too many requests) | Timed out immediately for 15 minutes |
| `Retry-After` longer than 15 minutes | The timeout lasts that long instead |
| Timeout ends | One trial request at a time; if it fails, immediately another 15 minutes |
| Connection failure or no network | Short back-off of 2 to 60 s; our side may be the problem, so no 15-minute timeout |
| HTTP 404 | Normal: remembered as "not available" |

- Timeouts survive restarts, so restarting the app cannot be used to hammer a server by accident.
- At most 4 requests run at once per server, and 2 per server for WMS services, which render every image on demand.
- While a server is timed out, nothing is sent to it, and other servers carry on.
- What is already cached keeps working, and coarser tiles stay on screen. A tile needs
  imagery to appear, so with an empty cache and the global imagery server timed out, new
  areas stay plain until it returns. Lines and labels for such a tile wait with it.
- The on-screen display names timed-out servers and the time remaining, along with the
  amount downloaded and how much came from cache.

## Local cache

- Tiles stay on disk until the size limit (10 GiB, `--cache-limit GB`) requires removing the
  longest-unused ones. Anything used in the last 6 hours is kept. `--cache-dir PATH` moves the cache.
- **Simultaneous requests for one file become one download.** Neighbouring tiles often
  share one coarser image or terrain file.
- **"Not available" is remembered for 30 days**, for HTTP 404 and for blank images.
- Damaged files are removed and fetched again rather than failing forever.
- Nothing is pre-fetched: only what a view needs is loaded, and queued tiles that leave the view are dropped.
- `--offline` sends no requests at all. `--cache-status` prints the cache size and server timeouts.

Answers of Wikipedia and Wikidata for the information box (`wikipedia/` in the cache) carry the time they were fetched and are used for a week; when offline or when the service cannot be reached, older ones are used too. Only answers that parse are kept. A click makes at most five such requests, one after another.

Map data does not refresh by itself: a cached tile is used until it is evicted.

## LAN cache server

Two ways to share, both off by default because they open a network port:

| Mode | Started with | Behaviour |
|---|---|---|
| Dedicated server | `orbis --serve-cache` | No window. Downloads what clients ask for through the rules above, so all of the LAN's traffic to a public server goes through one tracker |
| Shared app | `orbis --share-cache` | The normal app, also offering what it has cached to others, and using what others offer. Never downloads on behalf of another machine |

- Clients find servers by UDP broadcast with `--share-cache`, or use `--cache-server HOST:PORT`
  directly. A starting app asks who is there and waits briefly for answers before it
  downloads anything, so servers already on the LAN get used from the first tile.
- Servers speak plain HTTP on TCP port 47813 (`--cache-port PORT` changes it); discovery uses UDP port 47814.
- Standard tile URLs mean other programs can use it too, such as QGIS:
  `http://HOST:47813/tiles/{source}/{z}/{x}/{y}`.
- `http://HOST:47813/status` reports servers, timeouts and traffic as JSON.
- If a server is unreachable, clients skip it for a minute and carry on directly.
- If a dedicated server reports a public server as timed out, clients respect that too.

### Responses

| Status | Meaning |
|---|---|
| 200 | The tile |
| 404 | The public server does not have this tile |
| 503 + `Retry-After` | The public server is timed out or having problems |
| 504 | A shared app does not have the tile cached |
| 403 | The client is not on a private network |

### Safety

The server is meant for a trusted home or office network.

- Only clients with private, link-local or loopback addresses are answered.
- It accepts `GET` and `HEAD` only, and URLs are parsed into a known source and numbers, so
  they can never name a file path or an arbitrary upstream URL. It is not an open proxy.
- There is no encryption or login: anyone on the LAN can read cached tiles, or could pretend
  to be a cache server. Tile decoders treat all input as untrusted, so the worst expected
  result of a malicious peer is wrong map content.
- Data licences still apply to whatever is served, including credits for what is displayed.

## Other bodies

The Moon, Mars, Mercury, Ceres and Vesta each use one NASA Trek layer of plate carrée tiles, cached under
a folder per body (such as `nasa-trek-moon-wac/`) and requested two at a time from `trek.nasa.gov`, which
has its own pacing and overload pause. Earth's sources are never asked for while viewing another body, and
the other way round.

## Imagery by level of detail

Candidates for a tile are tried best first, within each source's approximate coverage box.
A 404 or a blank image falls through to the next. Once a coarse image of an area has turned
out empty, the service is not asked for anything finer inside it. The global base covers
everything else, magnified beyond its resolution.

| Source | Area | From zoom | To zoom | Licence or terms | Credit |
|---|---|---|---|---|---|
| Berlin TrueDOP | Berlin | 13 | 19 | DL-DE Zero 2.0 | Geoportal Berlin |
| NRW DOP | North Rhine-Westphalia | 13 | 19 | GovData terms | Geobasis NRW |
| LGLN DOP20 | Lower Saxony | 13 | 19 | CC BY 4.0 | LGLN (2026) CC BY 4.0 |
| Bavaria DOP40 | Bavaria | 13 | 18 | CC BY 4.0 | Bayerische Vermessungsverwaltung |
| basemap.at | Austria | 13 | 19 | CC BY 4.0 | Grundkarte: basemap.at |
| swisstopo SWISSIMAGE | Switzerland | 13 | 19 | Free under fair use | © swisstopo |
| IGN BD ORTHO | France | 13 | 19 | Etalab Open Licence by default | © IGN |
| PDOK Luchtfoto | Netherlands | 13 | 19 | No constraints stated | PDOK Luchtfoto |
| PNOA | Spain | 13 | 19 | CC BY 4.0 | PNOA CC BY 4.0 scne.es |
| USGS Imagery | USA | 13 | 16 | US public domain | USGS The National Map |
| VersaTiles | Rest of Germany, about 15 more European countries | 13 | 18 | Open licences per country | VersaTiles sources https://versatiles.org/sources/ |
| EOxCloudless (base) | World | 0 | 14 | CC BY-NC-SA 4.0 | See PROVENANCE.md |

Waiting for a slow service holds up only its own tiles. A worker never queues for a server
at its limit, or for a tile someone else is fetching: the job goes back in the queue and
the worker takes one that can go on, so terrain, map data and other imagery keep arriving.
The limit on requests per server is unchanged, and coarser imagery stays on screen meanwhile.

Terrain and map data stay on one server each, since both already vary in detail by zoom.
Credits on screen follow the sources in view.

The aerial roofs of 3D buildings use the same imagery, found the same way, for each building
tile (zoom 14) at the zoom of the finest ground drawn in it, at most 17. Where the ground near
the camera is finer, those zoom-17 images are requests of their own: up to 64 per building tile,
or 16 from VersaTiles, whose images are four tiles each. They are asked for after everything
else in view, and they are cached like any other tile, so a second visit costs nothing.

### Limitations

- Coverage is partial: most of the world stays at 10 m, including Italy, Poland and the UK.
  More services fit the same table, and users can add their own (see FEATURES.md, Map services).
- VersaTiles' images are 512 pixels and end at its zoom 17, which gives zoom 18 detail. Each
  tile uses a quarter of its parent image, so four tiles cost one download. Its metadata
  advertises zoom 19, but sampled tiles stopped at 17; recheck if that changes.
- VersaTiles publishes no usage terms for its public server, and it is one project's service.
  It gets two requests at a time, is asked only where no national service answers, and is
  worth contacting before Orbis is published.
- A service answering with one plain image by mistake hides its finer imagery of that area
  until the note expires after 30 days.
- A cache server on the LAN is asked directly, so a client's worker waits while that server
  gets a tile from a slow public service. Only public servers have the no-waiting rule.
- LGLN takes about a second per image, so a cold close view there takes much longer than
  elsewhere. Asking for 512-pixel images makes one request serve four tiles.
- LGLN asks for the year of retrieval in its credit. The year is fixed in the source list and
  needs updating with each release, and older cached tiles show the newer year.
- Images change at national borders and between zoom 12 and 13, and differ in date and colour.
- A tile crossing a coverage edge can show white or black where the service has no data.
- basemap.at prints the capture year as a watermark at zooms 17 and above.
