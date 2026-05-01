

# BobNote: A Sticky Notes Tool & Terminal Multiplexer

_BobNote_, named after my cat (Bob, short for Crowbar), is a sticky notes application I created because I ran out of space in my notebook. However, after a few weeks of tinkering, and the addition of "Shell Notes", which came from the need to already create a super basic terminal emulator for the tool to function, I have begun to use it as my main terminal multiplexer, which is why I am posting it here. This application works similarly to a Window Manager or desktop environment and supports both mouse and keyboard usage. You can see the gif below for a quick demo of BobNote in action:

<img width="1045" height="638" alt="test2" src="https://github.com/user-attachments/assets/efe451f3-2562-404d-b80a-7d92fa7743bb" />

_The Hotkey list is below this quick introduction to the functions and features of BobNote. The following screenshots show the basic usage of the BobNote app. I should also mention that claude was used in the development of this tool to speed things up a little. I hope that this application proves useful to you!_



---

# Basic Usage


<img width="1041" height="636" alt="test3" src="https://github.com/user-attachments/assets/aa8a627c-a9eb-4b44-ab68-f89b98bb3ed3" />


As can be seen above, there are multiple workspaces, which work similarly to any window manager that you may be familiar with. I tried to keep everything as convenient as possible with the design of this application, in the spirit of this I also included a ".bobrc" configuration file which is created in the home directory when _BobNote_ is run for the first time:

<img width="1858" height="966" alt="bobnoteconfig" src="https://github.com/user-attachments/assets/286bc537-3341-49d1-b395-0361b2c9642b" />

Now, to organize these notes, I was left with a decision. I settled on creating the "corkboard" interface. You can pin notes to this selection screen from any active workspace, and later pull them back from the corkboard to any workspace. This selection mechanism essentially works like any other file manager:

<img width="1048" height="641" alt="bobnote corkboard" src="https://github.com/user-attachments/assets/bc023319-480c-47f5-bbab-ccf1e4f3cd27" />


\
Some of my favourite features implemented so far include the screenshots, which can easily be copied and pasted for documentation and reporting purposes:
<p align="center">

  <img width="1041" height="636" alt="photo" src="https://github.com/user-attachments/assets/daca696a-723d-406d-8da1-f95563c6365f" />

  <img width="314" height="176" alt="bobnotephoto" src="https://github.com/user-attachments/assets/72c2e0b4-e234-4274-8939-22dcc834d868" />
</p>
  
I have also implemented NoteBooks! These work like regular books, you can add and remove pages within the corkboard interface to create easy to navigate and organize groups of any kinds of notes! These _notebooks_ can not only be identified by the page count and notebook title in the note's title area, but also by the "cover spine" shown on the side of all notbooks.

<p align="center">
  <img width="335" height="254" alt="bobnotebook" src="https://github.com/user-attachments/assets/63dd5976-a84f-4d4d-8a32-3d1810e57a86" />
</p>

_Note: If you enjoy BobNote, and have any work opportunities available, either remote or located in Victoria, B.C., please feel free to reach out to me at oatsecurity@gmail.com. Thank you very much for checking out my project!_ 

---

## Build Instructions

This application builds just like any rust application. Pre-compiled linux binaries will also be made available soon in case you would rather wait. To build this application, install `cargo` or `rustup` and build using this

```
$ cargo build --release
```

---


# BobNote Hotkeys

All keybindings are configurable in `~/.bobrc` unless marked *(fixed)*.

---

## Global — any context

These work regardless of what currently has focus.

| Key | Action |
|-----|--------|
| `F1` | Toggle the hint bar |
| `Alt+Q` | Quit BobNote |
| `Alt+B` | Open / close the corkboard |
| `Alt+N` | Create a new text note |
| `Alt+F` | Focus the topmost visible text note |
| `Alt+L` | New checklist (or focus topmost if one exists) |
| `Alt+T` | Create a new terminal note |
| `Alt+G` | Focus the topmost visible terminal note |
| `Alt+Space` | Cycle forward through terminal notes on this workspace |
| `Alt+O` | Focus the open notebook page / cycle open notebooks |
| `Alt+V` | Paste from system clipboard |
| `Alt+R` | Rename the active workspace |
| `Alt+←` / `Alt+→` | Switch to previous / next workspace |
| `Alt+=` | Add a new workspace |
| `Alt+-` | Remove the current workspace |
| `Shift+Insert` | Paste from clipboard into the background terminal |

---

## Background shell note (workspace terminal)

Active when a workspace's dedicated background terminal has focus (`Ctrl+E` from within it returns to main shell).

| Key | Action |
|-----|--------|
| `Ctrl+E` | Return to the main background shell |
| `PageUp` | Scroll up |
| `PageDown` | Scroll down |
| `Ctrl+V` | Enter screenshot mode |
| `Alt+←` / `Alt+→` | Switch workspace |
| `Alt+=` / `Alt+-` | Add / remove workspace |
| `Alt+I` | Start Logging to File |
| *(any other key)* | Sent to this workspace's shell |

---

## Note focused (all note types)

Active whenever any floating note has focus.

| Key | Action |
|-----|--------|
| `Ctrl+E` | Return focus to the background shell |
| `Ctrl+W` | Close / delete the note (moves to trash) |
| `Ctrl+P` | Pin note to / remove from corkboard |
| `Ctrl+F` | Toggle always-on-top |
| `Ctrl+T` | Rename the note |
| `Ctrl+S` | Open note settings (border, colour, wrap) |
| `Ctrl+G` | Assign note to a notebook |
| `Alt+H` / `Alt+←` | Move note left |
| `Alt+L` / `Alt+→` | Move note right |
| `Alt+K` / `Alt+↑` | Move note up |
| `Alt+J` / `Alt+↓` | Move note down |
| `Ctrl+Alt+H` / `Ctrl+Alt+←` | Shrink note width |
| `Ctrl+Alt+L` / `Ctrl+Alt+→` | Grow note width |
| `Ctrl+Alt+K` / `Ctrl+Alt+↑` | Shrink note height |
| `Ctrl+Alt+J` / `Ctrl+Alt+↓` | Grow note height |

---

## Text note focused

| Key | Action |
|-----|--------|
| `Tab` | Cycle to next text or checklist note on this workspace |
| `Ctrl+V` | Enter visual-select mode |
| `Ctrl+C` | Copy selection to clipboard |
| `Alt+C` | Copy selection to clipboard |
| *(any printable key)* | Edit note content |

---

## Checklist note focused

| Key | Action |
|-----|--------|
| `Tab` | Cycle to next text or checklist note on this workspace |
| `Ctrl+X` | Toggle the current checklist item `[ ]` / `[x]` |
| *(any printable key)* | Edit note content |

---

## Terminal note focused

| Key | Action |
|-----|--------|
| `Alt+Space` | Cycle to next terminal note on this workspace |
| `Ctrl+Y` | Snapshot terminal as a photo note |
| `Ctrl+B` | Send terminal to background (workspace shell) |
| `Ctrl+C` | Send SIGINT to the shell (pass-through) |
| `Alt+C` | Copy selected text to clipboard |
| `Alt+V` | Paste from clipboard |
| `Alt+I` | Start Logging to File |
| `PageUp` | Scroll terminal history up |
| `PageDown` | Scroll terminal history down |
| *(any other key)* | Sent directly to the terminal |

---

## Photo note focused

| Key | Action |
|-----|--------|
| `Ctrl+C` | Copy photo to clipboard |
| `Ctrl+T` | Rename |
| `Ctrl+P` | Pin to corkboard |
| `Ctrl+G` | Assign to notebook |
| `Ctrl+F` | Toggle always-on-top |
| `Ctrl+W` | Close / delete |

---

## Visual select mode (screenshot)

Entered with `Ctrl+V` from the background shell or a terminal note.

| Key | Action |
|-----|--------|
| `h` / `←` | Move selection left |
| `l` / `→` | Move selection right |
| `k` / `↑` | Move selection up |
| `j` / `↓` | Move selection down |
| `Enter` / `y` | Capture selection as a photo note |
| `Alt+C` | Copy selected text to clipboard |
| `Esc` | Exit screenshot mode |

---

## Text visual select mode

Entered with `Ctrl+V` from a text note.

| Key | Action |
|-----|--------|
| `h` / `←` | Move cursor left |
| `l` / `→` | Move cursor right |
| `k` / `↑` | Move cursor up |
| `j` / `↓` | Move cursor down |
| `w` | Jump forward one word |
| `b` | Jump backward one word |
| `0` | Jump to start of line |
| `$` | Jump to end of line |
| `y` / `Alt+C` | Yank / copy selection to clipboard |
| `Esc` | Exit visual select |

---

## Note settings popup

Opened with `Ctrl+S` while a note is focused.

| Key | Action |
|-----|--------|
| `Tab` / `↑` / `↓` | Cycle between sections (border toggle, border colour, background colour, text wrap) |
| `←` / `→` | Change colour |
| `Space` | Toggle border on/off (or toggle text wrap) |
| `Esc` / `Enter` | Close popup |

---

## Rename prompt (note or workspace)

Shown when renaming a note (`Ctrl+T`) or a workspace (`Alt+R`).

| Key | Action |
|-----|--------|
| *(any printable key)* | Type the new name |
| `Backspace` | Delete last character |
| `Enter` | Confirm rename |
| `Esc` | Cancel |

---

## Corkboard — main grid

Opened with `Alt+B`.

| Key | Action |
|-----|--------|
| `←` / `h` | Select item to the left |
| `→` / `l` | Select item to the right |
| `↑` / `k` | Select item above |
| `↓` / `j` | Select item below |
| `Enter` | Pick up note / open notebook folder / open terminal full-screen |
| `n` | Create a new notebook (prompts for name) |
| `a` | Assign selected note to a notebook |
| `t` | Rename selected note |
| `Ctrl+W` | Trash selected note or dissolve notebook |
| `Esc` / `Ctrl+K` | Close corkboard |

---

## Corkboard — notebook sub-grid

Active after pressing `Enter` on a notebook folder.

| Key | Action |
|-----|--------|
| `←` / `h` | Select page to the left |
| `→` / `l` | Select page to the right |
| `↑` / `k` | Select page above |
| `↓` / `j` | Select page below |
| `Enter` | Open selected page |
| `t` | Rename selected page note |
| `a` | Add a free corkboard note to this notebook |
| `r` / `Ctrl+W` | Remove selected page from notebook |
| `Ctrl+Left/Right/Up/Down` | Reorder selected page |
| `Esc` | Back to main corkboard grid |

---

## Corkboard — expanded terminal

Active when a terminal note is opened full-screen from the corkboard.

| Key | Action |
|-----|--------|
| `PageUp` / `PageDown` | Scroll terminal history |
| `Ctrl+E` | Pick up terminal note (move back to main view) |
| `Esc` | Return to corkboard grid |

---

## Notebook page (book mode)

Active when a note opened from a notebook is focused on the main view.

| Key | Action |
|-----|--------|
| `Tab` | Next page in this notebook |
| `Shift+Tab` | Previous page in this notebook |
| `Alt+O` | Cycle to the next open notebook |
| `Ctrl+R` | Remove this page from its notebook |
| `Ctrl+P` | Pin page back to corkboard |
| `Alt+P` | Toggle notebook persistence (floats across all workspaces) |
| `Ctrl+E` | Return to background shell |

---

## Customizing bindings

Edit `~/.bobrc` (created automatically on first launch with all defaults commented in).

```toml
# Example overrides
new_note         = "alt+n"
new_terminal     = "alt+t"
focus_terminal   = "alt+g"
rename_workspace = "alt+r"
return_to_shell  = "ctrl+e"
```

Supported modifiers: `alt`, `ctrl`.  
Supported keys: `a-z`, `0-9`, `f1–f12`, `esc`, `enter`, `tab`, `backspace`, `up`, `down`, `left`, `right`, `pageup`, `pagedown`, `home`, `end`, `insert`, `delete`.
