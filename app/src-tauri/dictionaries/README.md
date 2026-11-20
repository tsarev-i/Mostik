# Spelling dictionaries

Hunspell dictionaries that Mostik carries on Linux, where WebKitGTK checks spelling through enchant
and many systems have no dictionary for Russian or English. They are built into the Linux binary and
written to the app data folder at start (`src/spelling.rs`); the system's dictionaries are used too.

Taken from [wooorm/dictionaries](https://github.com/wooorm/dictionaries) at commit
`8cfea406b505e4d7df52d5a19bce525df98c54ab`, unchanged except for the file names (`index.aff` and
`index.dic` renamed by language). Modified versions, as the Russian license asks to mark them: that
project generates them from [LibreOffice/dictionaries](https://github.com/LibreOffice/dictionaries)
and normalizes them (UTF-8 encoding); the Russian `ru_RU.aff` also carries the 2012 change noted in
its license.

| Files | Source | License |
|---|---|---|
| `en_US.aff`, `en_US.dic` | `dictionaries/en`, SCOWL en_US 2020.12.07 | MIT and BSD, [LICENSE-en_US](LICENSE-en_US) |
| `ru_RU.aff`, `ru_RU.dic` | `dictionaries/ru`, by Alexander I. Lebedev | BSD, [LICENSE-ru_RU](LICENSE-ru_RU) |

The licenses and this file go with the dictionaries into the app data folder, so every copy of
Mostik that has the dictionaries has their notices too.
