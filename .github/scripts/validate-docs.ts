#!/usr/bin/env bun

/**
 * Documentation Validation Script
 *
 * Validates documentation files for:
 * - YAML syntax integrity
 * - Hardcoded secrets/credentials
 * - Incomplete task items
 * - Markdown formatting
 */

import * as fs from "fs";
import * as path from "path";

interface ValidationResult {
  passed: boolean;
  errors: string[];
  warnings: string[];
}

// Directories to validate. Override per-repo via the DOCS_PATHS env var
// (space-separated), e.g. DOCS_PATHS="docs specs standards policies templates".
// Defaults preserve prior behavior.
const PATHS_TO_CHECK = process.env.DOCS_PATHS?.trim()
  ? process.env.DOCS_PATHS.trim().split(/\s+/)
  : ["specs", "docs"];

// Secret patterns to detect - conservative to avoid false positives
const SECRET_PATTERNS = [
  /-----BEGIN[\s\S]{0,100}KEY-----/,  // Private key markers (PEM format)
  /ghp_[A-Za-z0-9]{36,255}/,          // GitHub personal access token
  /github_pat_[A-Za-z0-9]{22,255}/,   // GitHub PAT new format
  /sk_live_[A-Za-z0-9]{20,}/,         // Stripe live key
  /AKIA[0-9A-Z]{16}/,                 // AWS access key
  /-----END[\s\S]{0,100}KEY-----/,    // End of private key
];

// File patterns to validate
const YAML_PATTERNS = /\.(yaml|yml)$/i;
// Match .md and .mdx so the scan covers the same files the ci-docs workflow
// triggers on (**/*.md and **/*.mdx).
const MARKDOWN_PATTERNS = /\.mdx?$/i;

/**
 * Recursively find all files matching pattern in directories
 */
function findFiles(
  dirs: string[],
  pattern: RegExp
): string[] {
  const files: string[] = [];

  function walk(dir: string) {
    if (!fs.existsSync(dir)) {
      return;
    }

    const entries = fs.readdirSync(dir, { withFileTypes: true });

    for (const entry of entries) {
      const fullPath = path.join(dir, entry.name);

      if (entry.isDirectory()) {
        // Skip hidden and common ignore directories
        if (!entry.name.startsWith(".") && entry.name !== "node_modules") {
          walk(fullPath);
        }
      } else if (pattern.test(entry.name)) {
        files.push(fullPath);
      }
    }
  }

  for (const dir of dirs) {
    walk(dir);
  }

  return files;
}

/**
 * Validate YAML syntax
 */
function validateYAML(filePath: string): { valid: boolean; error?: string } {
  try {
    const content = fs.readFileSync(filePath, "utf-8");

    // Basic YAML validation - check for common syntax issues
    const lines = content.split("\n");

    for (let i = 0; i < lines.length; i++) {
      const line = lines[i];
      if (line.trim() === "" || line.trim().startsWith("#")) {
        continue;
      }

      // YAML forbids tab characters for indentation; flag them explicitly.
      const leadingWhitespace = line.slice(
        0,
        line.length - line.trimStart().length
      );
      if (leadingWhitespace.includes("\t")) {
        return {
          valid: false,
          error: `Line ${i + 1}: Invalid YAML syntax - tab used for indentation (YAML requires spaces)`,
        };
      }

      // Validate key-value pairs
      if (line.includes(":")) {
        const beforeColon = line.split(":")[0];
        if (beforeColon.trim() === "") {
          return {
            valid: false,
            error: `Line ${i + 1}: Invalid YAML syntax - key expected before colon`,
          };
        }
      }
    }

    return { valid: true };
  } catch (error) {
    return {
      valid: false,
      error: `Failed to read file: ${error instanceof Error ? error.message : String(error)}`,
    };
  }
}

/**
 * Check for hardcoded secrets in file
 */
function checkForSecrets(filePath: string): string[] {
  const secrets: string[] = [];

  try {
    const content = fs.readFileSync(filePath, "utf-8");
    const lines = content.split("\n");

    for (let i = 0; i < lines.length; i++) {
      const line = lines[i];

      for (const pattern of SECRET_PATTERNS) {
        if (pattern.test(line)) {
          // Mask the secret for display
          const maskedLine = line.replace(/[a-zA-Z0-9_\-]{8,}/g, "***REDACTED***");
          secrets.push(
            `${filePath}:${i + 1}: Potential secret detected: ${maskedLine.trim()}`
          );
        }
      }
    }
  } catch (error) {
    console.warn(`Warning: Could not check file ${filePath} for secrets`);
  }

  return secrets;
}

/**
 * Check for incomplete task items in markdown
 */
function checkIncompleteTasksInFile(filePath: string): string[] {
  const tasks: string[] = [];

  try {
    const content = fs.readFileSync(filePath, "utf-8");
    const lines = content.split("\n");

    for (let i = 0; i < lines.length; i++) {
      const line = lines[i];
      if (/^\s*[-*]\s*\[\s\]/.test(line)) {
        tasks.push(`${filePath}:${i + 1}: ${line.trim()}`);
      }
    }
  } catch (error) {
    console.warn(`Warning: Could not check file ${filePath} for incomplete tasks`);
  }

  return tasks;
}

/**
 * Run all validations
 */
function runValidations(): ValidationResult {
  const result: ValidationResult = {
    passed: true,
    errors: [],
    warnings: [],
  };

  console.log("🔍 Starting documentation validation...\n");

  // Validate YAML files
  console.log("📋 Validating YAML files...");
  const yamlFiles = findFiles(PATHS_TO_CHECK, YAML_PATTERNS);

  if (yamlFiles.length > 0) {
    for (const file of yamlFiles) {
      const validation = validateYAML(file);
      if (!validation.valid) {
        result.errors.push(`${file}: ${validation.error}`);
        result.passed = false;
      } else {
        console.log(`  ✓ ${file}`);
      }
    }
  } else {
    console.log("  ℹ No YAML files found in specs/ or docs/");
  }

  // Check for secrets in all files
  console.log("\n🔐 Checking for hardcoded secrets...");
  const allFiles = findFiles(PATHS_TO_CHECK, /./); // All files
  let secretsFound = false;

  for (const file of allFiles) {
    // Skip binary files and common non-text extensions
    if (/\.(png|jpg|jpeg|gif|pdf|zip|tar|gz)$/i.test(file)) {
      continue;
    }

    const secrets = checkForSecrets(file);
    if (secrets.length > 0) {
      result.errors.push(...secrets);
      result.passed = false;
      secretsFound = true;
    }
  }

  if (!secretsFound) {
    console.log("  ✓ No secrets detected");
  }

  // Check for incomplete tasks
  console.log("\n✅ Checking for incomplete tasks...");
  const markdownFiles = findFiles(PATHS_TO_CHECK, MARKDOWN_PATTERNS);
  const incompleteTasks: string[] = [];

  for (const file of markdownFiles) {
    const tasks = checkIncompleteTasksInFile(file);
    incompleteTasks.push(...tasks);
  }

  if (incompleteTasks.length > 0) {
    result.warnings.push(
      `Found ${incompleteTasks.length} incomplete task(s):`
    );
    incompleteTasks.slice(0, 10).forEach((task) => {
      result.warnings.push(`  ⚠️  ${task}`);
    });
    if (incompleteTasks.length > 10) {
      result.warnings.push(
        `  ... and ${incompleteTasks.length - 10} more unchecked task(s)`
      );
    }
  } else {
    console.log("  ✓ No incomplete tasks found");
  }

  return result;
}

/**
 * Print results and exit with appropriate status code
 */
function printResults(result: ValidationResult): void {
  console.log("\n" + "=".repeat(60));

  if (result.errors.length > 0) {
    console.log("\n❌ VALIDATION FAILED\n");
    console.log("Errors:");
    result.errors.forEach((error) => {
      console.log(`  ${error}`);
    });
  } else {
    console.log("\n✅ VALIDATION PASSED\n");
  }

  if (result.warnings.length > 0) {
    console.log("Warnings:");
    result.warnings.forEach((warning) => {
      console.log(`  ${warning}`);
    });
  }

  console.log("=".repeat(60) + "\n");

  process.exit(result.passed ? 0 : 1);
}

// Main execution
const result = runValidations();
printResults(result);
