import AppKit
import Combine
import Foundation
import UniformTypeIdentifiers

@MainActor
final class DocumentStore: ObservableObject {
    @Published var document: SemanticDocument?
    @Published var semanticURL: URL?
    @Published var pdfURL: URL?
    @Published var selectedBlockID: String?
    @Published var errorMessage: String?
    @Published var isParsingPDF = false
    @Published var statusMessage: String?

    func loadFromArguments() {
        let arguments = Array(CommandLine.arguments.dropFirst())
        guard !arguments.isEmpty else {
            return
        }

        var pendingPDF: URL?

        for argument in arguments {
            let url = URL(fileURLWithPath: argument)
            if url.pathExtension.lowercased() == "json" {
                loadSemanticDocument(from: url)
            } else if url.pathExtension.lowercased() == "pdf" {
                pendingPDF = url
            }
        }

        if let pendingPDF {
            if document == nil {
                parsePDF(at: pendingPDF)
            } else {
                pdfURL = pendingPDF
            }
        }
    }

    func openSemanticJSON() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [.json]
        panel.canChooseFiles = true
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false

        if panel.runModal() == .OK, let url = panel.url {
            loadSemanticDocument(from: url)
        }
    }

    func openPDF() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [.pdf]
        panel.canChooseFiles = true
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false

        if panel.runModal() == .OK, let url = panel.url {
            parsePDF(at: url)
        }
    }

    func loadSemanticDocument(from url: URL) {
        do {
            let data = try Data(contentsOf: url)
            let decoder = JSONDecoder()
            let loaded = try decoder.decode(SemanticDocument.self, from: data)
            semanticURL = url
            document = loaded
            selectedBlockID = loaded.blocks.first?.id
            errorMessage = nil
            statusMessage = "Loaded \(url.lastPathComponent)"

            if pdfURL == nil, let sourcePath = loaded.metadata.sourcePath {
                let sourceURL = URL(fileURLWithPath: sourcePath)
                if FileManager.default.fileExists(atPath: sourceURL.path) {
                    pdfURL = sourceURL
                }
            }
        } catch {
            errorMessage = "Failed to load semantic JSON: \(error.localizedDescription)"
        }
    }

    func jumpToBlock(_ blockID: String?) {
        selectedBlockID = blockID
    }

    func parsePDF(at url: URL) {
        pdfURL = url
        statusMessage = "Parsing PDF…"
        isParsingPDF = true
        errorMessage = nil

        Task.detached(priority: .userInitiated) { [self] in
            do {
                let data = try await self.runParser(for: url)
                let decoder = JSONDecoder()
                let loaded = try decoder.decode(SemanticDocument.self, from: data)
                await MainActor.run {
                    self.document = loaded
                    self.semanticURL = nil
                    self.selectedBlockID = loaded.blocks.first?.id
                    self.statusMessage = "Parsed \(url.lastPathComponent)"
                    self.isParsingPDF = false
                }
            } catch {
                await MainActor.run {
                    self.document = nil
                    self.isParsingPDF = false
                    self.statusMessage = nil
                    self.errorMessage = "Failed to parse PDF: \(error.localizedDescription)"
                }
            }
        }
    }

    nonisolated private func runParser(for pdfURL: URL) async throws -> Data {
        let parserExecutable = try parserExecutableURL()
        let process = Process()
        process.executableURL = parserExecutable
        process.arguments = [pdfURL.path, "--format", "semantic-json"]
        process.currentDirectoryURL = parserManifestURL().deletingLastPathComponent()

        let output = Pipe()
        let error = Pipe()
        process.standardOutput = output
        process.standardError = error

        if parserExecutable.lastPathComponent == "cargo" {
            process.arguments = [
                "run",
                "--manifest-path",
                parserManifestURL().path,
                "--",
                pdfURL.path,
                "--format",
                "semantic-json",
            ]
        }

        try process.run()

        // Drain both pipes before waiting: parser output is far larger than the pipe buffer,
        // so waiting first would deadlock with the parser blocked on a full stdout pipe.
        let stderrTask = Task.detached { error.fileHandleForReading.readDataToEndOfFile() }
        let stdout = output.fileHandleForReading.readDataToEndOfFile()
        let stderr = await stderrTask.value
        process.waitUntilExit()

        guard process.terminationStatus == 0 else {
            let parserError = String(data: stderr, encoding: .utf8)?.trimmingCharacters(in: .whitespacesAndNewlines)
            throw NSError(
                domain: "FluidPaperReader.Parser",
                code: Int(process.terminationStatus),
                userInfo: [
                    NSLocalizedDescriptionKey: parserError?.isEmpty == false
                        ? parserError!
                        : "Parser exited with status \(process.terminationStatus)",
                ]
            )
        }

        return stdout
    }

    nonisolated private func parserExecutableURL() throws -> URL {
        let fileManager = FileManager.default
        let projectRoot = try projectRootURL()
        let candidates = [
            ProcessInfo.processInfo.environment["FLUIDPAPER_CLI"].map(URL.init(fileURLWithPath:)),
            projectRoot.appendingPathComponent("target/debug/fluidpaper"),
        ].compactMap { $0 }

        for candidate in candidates where fileManager.isExecutableFile(atPath: candidate.path) {
            return candidate
        }

        guard let cargoPath = findExecutable(named: "cargo") else {
            throw NSError(
                domain: "FluidPaperReader.Parser",
                code: 1,
                userInfo: [
                    NSLocalizedDescriptionKey:
                        "Could not find the FluidPaper parser binary or cargo. Build the Rust CLI first with `cargo build`.",
                ]
            )
        }

        return cargoPath
    }

    nonisolated private func parserManifestURL() -> URL {
        (try? projectRootURL().appendingPathComponent("Cargo.toml"))
            ?? URL(fileURLWithPath: "Cargo.toml")
    }

    nonisolated private func findExecutable(named name: String) -> URL? {
        let environmentPath = ProcessInfo.processInfo.environment["PATH"] ?? ""
        for path in environmentPath.split(separator: ":") {
            let candidate = URL(fileURLWithPath: String(path)).appendingPathComponent(name)
            if FileManager.default.isExecutableFile(atPath: candidate.path) {
                return candidate
            }
        }
        return nil
    }

    nonisolated private func projectRootURL() throws -> URL {
        let executableURL = URL(fileURLWithPath: Bundle.main.executablePath ?? CommandLine.arguments[0]).standardizedFileURL

        for ancestor in executableURL.deletingLastPathComponent().ancestors {
            let manifest = ancestor.appendingPathComponent("Cargo.toml")
            if FileManager.default.fileExists(atPath: manifest.path) {
                return ancestor
            }
        }

        throw NSError(
            domain: "FluidPaperReader.Parser",
            code: 2,
            userInfo: [
                NSLocalizedDescriptionKey:
                    "Could not locate the FluidPaper project root from the reader executable.",
            ]
        )
    }
}

private extension URL {
    var ancestors: [URL] {
        var urls = [self]
        var current = self

        while current.path != "/" {
            current = current.deletingLastPathComponent()
            urls.append(current)
        }

        return urls
    }
}
