# AetherOS Multi-Window Window Manager (`WINDOW_MANAGER.md`)

## 1. Window Management Architecture (`kernel/src/gui/mod.rs`)

The AetherOS Window Manager manages overlapping windows across 4 virtual workspaces with deterministic z-order stacking (`z_order: Vec<u32>`).

### Supported Window Operations
- **Focus & Z-Order Raising**: Clicking anywhere inside an unfocused window raises it to the top of `z_order` and highlights its active border and title bar with the user's selected accent color.
- **Title-Bar Dragging (`DragMode::Move`)**: Pressing and holding the left mouse button on any window's 24 px title bar moves the window smoothly across the screen while clamping coordinates (`x in 4..width-96`, `y in 28..height-68`) so windows can never be lost off-screen.
- **Corner Resizing (`DragMode::Resize`)**: Dragging the bottom-right `14x14` resize grip resizes the window dynamically (clamped to minimum `280x160`).
- **Minimize (`[-]`, Yellow Button)**: Hides the window from the workspace canvas while keeping its taskbar pill active for one-click restoration.
- **Maximize & Restore (`[+]`, Green Button)**: Expands the window to fill the usable desktop area (`104, 28, width-108, height-64`) while saving `saved_rect` so clicking `[+]` again restores the previous size and position.
- **Close (`[X]`, Red Button)**: Closes the window; any closed application can be re-launched at any time from the Desktop Icons or the `[AETHER]` Start Menu Launcher.
- **Virtual Workspaces (`WS1..WS4`)**: Each window is tagged with a `workspace: u8` index (`0..3`) and automatically moves to the active workspace when launched or restored from the taskbar.
