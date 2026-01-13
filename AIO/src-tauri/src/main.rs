// src-tauri/src/main.rs
// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// Import necessary modules from the standard library and external crates
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::env; // For environment variables (optional fallback)

// Define the Assistant struct matching the frontend structure
#[derive(Serialize, Deserialize, Clone)]
struct Assistant {
    id: String,
    name: String,
    // Add other fields as needed
}

// --- Commands ---

/// Command to load assistants from a JSON file.
/// In Tauri v2, the macro is simpler: #[tauri::command]
#[tauri::command]
async fn load_assistants() -> Result<Vec<Assistant>, String> { // Added 'async'
    let file_path = get_data_file_path()?;

    // Check if the file exists
    if !file_path.exists() {
        println!(
            "Data file does not exist at {:?}, returning empty list.",
            file_path
        );
        // Return an empty list if the file doesn't exist yet.
        // The frontend handles creating the initial assistant.
        return Ok(vec![]);
    }

    // Read the file contents
    let contents = fs::read_to_string(&file_path)
        .map_err(|e| format!("Failed to read file '{:?}': {}", file_path, e))?;

    // Parse the JSON content into a Vec<Assistant>
    let assistants: Vec<Assistant> = serde_json::from_str(&contents)
        .map_err(|e| format!("Failed to parse JSON from '{:?}': {}", file_path, e))?;

    Ok(assistants)
}

/// Command to save the list of assistants to a JSON file.
#[tauri::command]
async fn save_assistants(assistants: Vec<Assistant>) -> Result<(), String> { // Added 'async'
    let file_path = get_data_file_path()?;

    // Ensure the parent directory exists
    if let Some(parent) = file_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create directory '{:?}': {}", parent, e))?;
    }

    // Serialize the assistants vector to a pretty-printed JSON string
    let json = serde_json::to_string_pretty(&assistants)
        .map_err(|e| format!("Failed to serialize assistants to JSON: {}", e))?;

    // Write the JSON string to the file
    fs::write(&file_path, json)
        .map_err(|e| format!("Failed to write file '{:?}': {}", file_path, e))?;

    println!("Assistants successfully saved to {:?}", file_path);
    Ok(())
}

/// Helper function to determine the path for the assistants data file.
/// Uses the system's configuration directory provided by the `dirs` crate.
fn get_data_file_path() -> Result<PathBuf, String> {
    // Attempt to get the configuration directory using the `dirs` crate
    match dirs::config_dir() {
        Some(mut path) => {
            // Append your app's specific folder and filename
            // TODO: Customize these identifiers for your app
            path.push("YourCompanyNameOrName"); // e.g., "MyCompany"
            path.push("YourAppName");          // e.g., "MyAwesomeAIApp"
            path.push("assistants.json");
            Ok(path)
        }
        None => {
            // Fallback mechanism if dirs::config_dir fails (rare)
            eprintln!("Warning: Could not determine config directory. Using current working directory.");
            let mut path = env::current_dir()
                .map_err(|e| format!("Failed to get current directory: {}", e))?;
            path.push("assistants_fallback.json"); // Different name to avoid confusion
            Ok(path)
        }
    }
}

// --- End Commands ---

fn main() {
    tauri::Builder::default()
        // Register the custom commands so they can be invoked from the frontend
        // Note: Function names are passed directly now in v2
        .invoke_handler(tauri::generate_handler![
            load_assistants,
            save_assistants
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}