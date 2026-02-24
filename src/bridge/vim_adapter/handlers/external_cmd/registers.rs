//! Register operations: ListRegisters.

use crate::bridge::vim_wrapper::VimController;
use godot::prelude::*;

impl VimController {
    pub(super) fn handle_list_registers(&mut self) {
        let mut output = String::new();
        output.push_str("--- Registers ---\n");

        // Sort registers for consistent display
        let mut regs: Vec<(char, &vim_core::domain::shared_str::SharedStr)> =
            self.engine.register_entries().collect();
        regs.sort_by_key(|(k, _)| *k);

        for (name, content) in regs {
            // Escape newlines for display
            let display_content: String = content.replace('\n', "^J");
            let display_content = if display_content.len() > 50 {
                format!("{}...", &display_content[..47])
            } else {
                display_content
            };
            output.push_str(&format!("\"{}   {}\n", name, display_content));
        }

        godot_print!("{}", output);
        self.show_cmdline_message("Registers listed in Output panel");
    }
}
