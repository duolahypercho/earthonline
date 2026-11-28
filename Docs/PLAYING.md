# Playing Earth Online

Earth Online is San Francisco built from open data: every street, building
footprint, hill and shoreline. You can walk it, drive it and fly over it.
It is in early development, so many systems are first versions and some
content is missing.

This guide is for the downloadable builds on the
[Releases page](https://github.com/duolahypercho/earthonline/releases). To
build from source instead, see the [README](../README.md#quick-start).

## What you need

- **Windows** 10 or 11, 64-bit, with a GPU that supports DirectX 12 or
  Vulkan.
- **macOS** on Apple silicon (M1 or later).
- **Linux**, 64-bit, with a Vulkan driver, and Ubuntu 24.04's C library
  or newer.
- **Memory.** 16 GB is comfortable; the game used about 7 GB on a first
  start on an Apple silicon Mac.
- **Disk space.** About 8 GB in all. The zip is about 250 MB, and about
  600 MB unzipped. On the first start, the game prepares models, building
  tiles and textures and saves them inside its own folder, in
  `native/cache/`, which grew to about 7 GB in the same test.

## Start the game

1. Download the zip for your computer and unzip it anywhere you can write
   to, for example your Downloads folder. Keep the folder as it is: the
   program reads the `public/` and `native/` folders beside it.
2. Start it:
   - **Windows:** double-click `play.bat`. If Windows shows "Windows
     protected your PC", choose **More info**, then **Run anyway**. The
     build is not code-signed.
   - **macOS:** double-click `play.command`. The build is not signed by
     Apple, so the first time macOS says it cannot verify the program.
     Choose **Done**, then open **System Settings > Privacy & Security**,
     scroll down to the message about `play.command`, choose
     **Open Anyway**, and confirm. Once it is open, the script clears the
     download flag from the game's two programs, `eo-app` and `eo-server`,
     so macOS does not stop them as well. To do all of this from Terminal
     instead:

     ```sh
     cd ~/Downloads/EarthOnline-*-macos-arm64
     xattr -dr com.apple.quarantine .
     ./play.command
     ```

   - **Linux:** run `./play.sh` from a terminal in the folder.
3. Wait for the first start. The window opens after about 15 seconds, and the
   terminal prints what it is preparing while the buildings fill in, which
   took about 4 minutes in the test above. Later starts read the saved
   files and are much quicker.

## Controls

On foot and driving:

| Input | On foot | Driving |
| --- | --- | --- |
| Click the window | Capture the mouse | Capture the mouse |
| Mouse | Look | Look |
| W A S D | Move | W accelerates, S holds the service brake, A/D steer |
| R | Sleep when beside the bed | Switch Drive/Reverse at a stop |
| Shift | Run; hold it going straight ahead to sprint | |
| C or Ctrl | Crouch | Look behind (C) |
| Space | Jump | Handbrake |
| F | Get in a car within reach | Get out |
| V | Place a new car ahead | |
| Esc | Release the mouse; press again to quit | |

The window title shows the frame rate, the mode and the speed.

## Other ways to start

Run the program from a terminal in the game's folder (`eo-app.exe` on
Windows):

| Command | What |
| --- | --- |
| `./eo-app --play` | Walk and drive (what the launchers start) |
| `./eo-app` | Fly over the city. Hold the right mouse button to look, W A S D to move, Q and E for down and up, Shift and Ctrl for faster and slower |
| `./eo-app --ui` | Fly over the city with the title screen, settings, map and pause menu |
| `./eo-app --play --beaver-house` | Start outside a furnished house at 21 Beaver Street. Press E at the door handle to open it |

Anything after the launcher's name is passed on as well, for example
`./play.command --beaver-house`.

To play together on one network, one person starts the server and everyone
connects to it:

```sh
./eo-server --bind 0.0.0.0:47800
./eo-app --connect <server address>:47800 --name <your name>
```

The server's port, 47800 by default, must be reachable from the other
computers.

Settings changed in `--ui` are saved in `%APPDATA%\Earth Online\settings.json`
on Windows and in `~/.config/earth-online/settings.json` on macOS and Linux.

## If something goes wrong

- **"could not find public/data/sf/sf-city.json.gz"**: the program was moved
  out of its folder, or the zip was not fully extracted. Start it from the
  unzipped folder, next to `public/` and `native/`.
- **No window, or a graphics error**: update your graphics driver. The game
  needs DirectX 12 or Vulkan on Windows, Metal on macOS and Vulkan on Linux.
- **Low frame rate**: the game is tuned on desktop GPUs, and on Macs it often
  runs below 60 frames a second. `./eo-app --play --quality low` lowers the
  detail.
- **Out of disk space on the first start**: free a few gigabytes and start
  again. The files saved so far are kept.
- **No sound, no people on the sidewalks, grey boxes for street lamps,
  signals and benches**: sound, street people, scanned surfaces, the far
  shore's relief and most street-furniture models need extra files that are
  not in the zip yet, so these are off in the downloadable builds.

To report a problem, open an issue on the
[project's GitHub page](https://github.com/duolahypercho/earthonline/issues).
Include what the terminal printed and your operating system and GPU.

## Licences

The game's code is MIT licensed. The map data is not: the OpenStreetMap-derived
files are © OpenStreetMap contributors under the Open Database License 1.0,
and the other data, models, textures and fonts keep their own terms.
`README.md`, `LICENSE`, `LICENSES/` and `THIRD_PARTY_NOTICES.md` in this
folder list them, and each data file in `public/data/sf/` has a `NOTICE`
beside it.
