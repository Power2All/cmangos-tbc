# C(ontinued)-MaNGOS (Rust) -- README
[![Windows](../../actions/workflows/windows.yml/badge.svg)](../../actions/workflows/windows.yml) [![Ubuntu](../../actions/workflows/ubuntu.yml/badge.svg)](../../actions/workflows/ubuntu.yml) [![MacOS](../../actions/workflows/macos.yml/badge.svg)](../../actions/workflows/macos.yml)

This file is part of the CMaNGOS Project, modified for the Rust implementation. See [AUTHORS](AUTHORS.md) and [COPYRIGHT](COPYRIGHT.md) files for Copyright information

## Welcome to C(ontinued)-MaNGOS Rust Implementation

CMaNGOS is a free project with the following goal:

**Doing Emulation Right!**

This means, we want to focus on:

* Doing
    * This project is focused on developing software!
    * Also there are many other aspects that need to be done and are considered equally important.
    * Anyone who wants to do stuff is very welcome to do so!

* Emulation
    * This project is about developing a server software that is able to emulate a well known MMORPG service.

* Right
    * Our goal must always be to provide the best code that we can.
    * Being 'right' is defined by the behaviour of the system we want to emulate.
    * Developing things right also includes documenting and discussing _how_ to do things better, hence...
    * Learning and teaching are very important in our view, and must always be a part of what we do.

To be able to accomplish these goals, we support and promote:

* Freedom
    * of our work: Our work - including our code - is released under the GPL. So everybody is free to use and contribute to this open source project.
    * for our developers and contributors on things that interest them. No one here is telling anybody _what_ to do. If you want somebody to do something for you, pay them, but we are here to enjoy.
    * to have FUN with developing.

* A friendly environment
    * We try to leave personal issues behind us.
    * We only argue about content and not about thin air!
    * We follow the [Netiquette](http://tools.ietf.org/html/rfc1855).

-- The C(ontinued)-MaNGOS Team!

## Further information

You can find further information about CMaNGOS at the following places:
* [CMaNGOS Discord](https://discord.gg/Dgzerzb)
* [GitHub repositories](https://github.com/cmangos/)
* [Issue tracker](https://github.com/cmangos/issues/issues)
* [Pull Requests](https://github.com/cmangos/mangos-tbc/pulls)
* [Wiki](https://github.com/cmangos/issues/wiki) with additional information on installation
* [Contributing Guidelines](CONTRIBUTING.md)
* Documentation can be found in the doc/ subdirectory and on the GitHub wiki

## License

CMaNGOS is free software; you can redistribute it and/or modify
it under the terms of the GNU General Public License as published by
the Free Software Foundation; either version 2 of the License, or
(at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU General Public License for more details.

You should have received a copy of the GNU General Public License
along with this program; if not, write to the Free Software
Foundation, Inc., 59 Temple Place, Suite 330, Boston, MA  02111-1307  USA

You can find the full license text in the file [COPYING](COPYING) delivered with this package.

### Exceptions to GPL

World of Warcraft® ©2004 Blizzard Entertainment, Inc. All rights reserved.
World of Warcraft® content and materials mentioned or referenced are copyrighted by
Blizzard Entertainment, Inc. or its licensors.
World of Warcraft, WoW, Warcraft, The Frozen Throne, The Burning Crusade, Wrath of the Lich King,
Cataclysm, Mists of Pandaria, Ashbringer, Dark Portal, Darkmoon Faire, Frostmourne, Onyxia's Lair,
Diablo, Hearthstone, Heroes of Azeroth, Reaper of Souls, Starcraft, Battle Net, Blizzcon, Glider,
Blizzard and Blizzard Entertainment are trademarks or registered trademarks of
Blizzard Entertainment, Inc. in the U.S. and/or other countries.

Any World of Warcraft® content and materials mentioned or referenced are copyrighted by
Blizzard Entertainment, Inc. or its licensors.
CMaNGOS project is not affiliated with Blizzard Entertainment, Inc. or its licensors.

Some third-party libraries CMaNGOS uses have other licenses, that must be
upheld.  These libraries are located within the dep/ directory

In addition, as a special exception, the CMaNGOS project
gives permission to link the code of its release of MaNGOS with the
OpenSSL project's "OpenSSL" library (or with modified versions of it
that use the same license as the "OpenSSL" library), and distribute
the linked executables.  You must obey the GNU General Public License
in all respects for all of the code used other than "OpenSSL".  If you
modify this file, you may extend this exception to your version of the
file, but you are not obligated to do so.  If you do not wish to do
so, delete this exception statement from your version.

### Additional to the Rust Implementation

The code generated, has been done partly by AI (Claude.AI), and further modified and tweaked by me (Power2All). This project started just as a experiment, to see how far we can use Claude.AI, to see if we can convert the C++ code into Rust, and how close we can get to how CMaNGOS works with Rust code only.

This project has been started after letting known to one of the developers (Killerwife), which blessed my soul for attempting this blasphemy, as I told him this is purely a experiment on my end, as I've been working with Rust for quiet a while.

Thus, this project should only be acknowledged as a "fun experiment", and nothing more as of this moment. Anything pushed here is done on my own and is based on the original source code from the [CMaNGOS Github](https://github.com/cmangos/), respectively the Burning Crusade code.

#### Windows Requirements

* Rust 1.85+ (Stable or Nightly) 2024 Edition
* [Optional] Visual Studio Build Tools (MSVC Target) needed for cl.exe (C++ compiler) and Windows SDK
* [Optional Alternative Compiler] MinGW-w64 (GNU Target)

#### Linux/OSX Requirements

* Rust 1.85+ (Stable or Nightly) 2024 Edition
* GCC/G++
  * Debian/Ubuntu: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh && sudo apt install build-essential g++`
  * Fedora/RHEL: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh && sudo dnf groupinstall "Development Tools" && sudo dnf install gcc-c++`
  * Arch: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh && sudo pacman -S base-devel gcc`
  * OSX: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh && xcode-select --install`

#### Build Commands

Default (no C++ needed) — builds extractors + realmd:
`cargo build --release`

With navmesh generation (needs C++ compiler):
`cargo build --release --features recast`

Just the extractors binary with recast:
`cargo build --release -p extractors --features recast`

#### Extractors Run Commands

Copy the extractors binary to the folder where World of Warcraft game resides in, and the following commands should work.
Depends on what operating system you run this, but as example I show the Windows commands:

Here is an example to extract everything:
`cd <WoW TBC Client>`
`extractors.exe map-dbc -o work`
`extractors.exe vmap-extract -d Data/ -o work -l`
`extractors.exe vmap-assemble work/Buildings work/vmaps`
`extractors.exe move-map-gen --workdir work`