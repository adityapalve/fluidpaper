import SwiftUI

private struct PresentedSection: Identifiable {
    let id: String
    let title: String
    let level: Int
    let pageStart: Int
    let blocks: [Block]
}

private struct ReaderPalette {
    let canvas: Color
    let card: Color
    let text: Color
    let subtle: Color
    let divider: Color
    let pdfChrome: Color
}

struct ReaderView: View {
    @EnvironmentObject private var store: DocumentStore
    @Environment(\.colorScheme) private var colorScheme
    @State private var fontSize: Double = 19
    @State private var lineSpacing: Double = 8
    @State private var horizontalPadding: Double = 60
    @State private var showPDFPreview = true

    private var palette: ReaderPalette {
        if colorScheme == .dark {
            return ReaderPalette(
                canvas: Color(red: 0.10, green: 0.09, blue: 0.08),
                card: Color(red: 0.16, green: 0.15, blue: 0.13),
                text: Color(red: 0.95, green: 0.93, blue: 0.89),
                subtle: Color(red: 0.70, green: 0.68, blue: 0.63),
                divider: Color.white.opacity(0.10),
                pdfChrome: Color(red: 0.18, green: 0.19, blue: 0.21)
            )
        }

        return ReaderPalette(
            canvas: Color(red: 0.985, green: 0.976, blue: 0.956),
            card: Color.white.opacity(0.9),
            text: Color(red: 0.12, green: 0.12, blue: 0.11),
            subtle: Color(red: 0.44, green: 0.42, blue: 0.38),
            divider: Color.black.opacity(0.06),
            pdfChrome: Color(red: 0.93, green: 0.94, blue: 0.95)
        )
    }

    var body: some View {
        NavigationSplitView {
            sidebar
        } detail: {
            detailView
        }
        .navigationTitle(store.document?.metadata.title ?? "FluidPaper Reader")
        .toolbar {
            ToolbarItemGroup {
                Button("Open JSON") { store.openSemanticJSON() }
                Button("Open PDF") { store.openPDF() }
                Toggle("Source Page", isOn: $showPDFPreview)
            }
        }
        .task {
            store.loadFromArguments()
        }
        .alert("Reader Error", isPresented: .constant(store.errorMessage != nil), actions: {
            Button("OK") { store.errorMessage = nil }
        }, message: {
            Text(store.errorMessage ?? "")
        })
        .overlay(alignment: .bottom) {
            if store.isParsingPDF || store.statusMessage != nil {
                HStack(spacing: 10) {
                    if store.isParsingPDF {
                        ProgressView()
                            .controlSize(.small)
                    }
                    Text(store.statusMessage ?? "Working…")
                        .font(.caption)
                }
                .padding(.horizontal, 14)
                .padding(.vertical, 10)
                .background(.ultraThinMaterial)
                .clipShape(Capsule())
                .padding(.bottom, 18)
            }
        }
    }

    private var sidebar: some View {
        List(selection: Binding(get: {
            store.selectedBlockID
        }, set: { newValue in
            store.jumpToBlock(newValue)
        })) {
            if let document = store.document {
                if let title = displayTitle(for: document) {
                    SwiftUI.Section("Document") {
                        Text(title).font(.headline)
                        let authors = displayAuthors(for: document)
                        if !authors.isEmpty {
                            Text(authors.joined(separator: ", "))
                                .font(.subheadline)
                                .foregroundStyle(palette.subtle)
                        }
                    }
                }

                SwiftUI.Section("Sections") {
                    ForEach(presentedSections(for: document)) { section in
                        if let firstBlock = section.blocks.first {
                            Button {
                                store.jumpToBlock(firstBlock.id)
                            } label: {
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(section.title)
                                        .foregroundStyle(palette.text)
                                    Text("Page \(section.pageStart)")
                                        .font(.caption)
                                        .foregroundStyle(palette.subtle)
                                }
                            }
                            .buttonStyle(.plain)
                        }
                    }
                }

                if !document.diagnostics.warnings.isEmpty {
                    SwiftUI.Section("Warnings") {
                        ForEach(document.diagnostics.warnings, id: \.self) { warning in
                            Text(warning)
                                .font(.caption)
                                .foregroundStyle(palette.subtle)
                        }
                    }
                }
            } else {
                ContentUnavailableView("Open semantic JSON", systemImage: "doc.text.magnifyingglass", description: Text("Load the Rust parser output to render a reflowed article view."))
            }
        }
        .frame(minWidth: 260)
        .scrollContentBackground(.hidden)
        .background(palette.card)
    }

    @ViewBuilder
    private var detailView: some View {
        if let document = store.document {
            HSplitView {
                readerColumn(document)
                if showPDFPreview {
                    pdfColumn(document)
                        .frame(minWidth: 320, idealWidth: 420)
                }
            }
            } else {
                ContentUnavailableView("Open a PDF or semantic JSON", systemImage: "book.pages", description: Text("The reader can now parse a PDF directly through the Rust engine, or load a previously exported semantic JSON file."))
            }
        }

    private func readerColumn(_ document: SemanticDocument) -> some View {
        let sections = presentedSections(for: document)

        return ScrollViewReader { proxy in
            ScrollView {
                VStack(alignment: .leading, spacing: 28) {
                    readerControls
                    header(document)
                    ForEach(sections) { section in
                        sectionView(section)
                    }
                }
                .frame(maxWidth: 760, alignment: .leading)
                .padding(.vertical, 30)
                .padding(.horizontal, horizontalPadding)
                .frame(maxWidth: .infinity, alignment: .top)
            }
            .background(palette.canvas)
            .onChange(of: store.selectedBlockID) { _, blockID in
                guard let blockID else { return }
                withAnimation(.easeInOut(duration: 0.2)) {
                    proxy.scrollTo(blockID, anchor: .top)
                }
            }
        }
    }

    private var readerControls: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Reading Controls")
                .font(.caption)
                .foregroundStyle(palette.subtle)

            HStack {
                labeledSlider(
                    title: "Font Size",
                    valueLabel: "\(Int(fontSize)) pt",
                    value: $fontSize,
                    range: 14...30,
                    step: 1
                )
                labeledSlider(
                    title: "Line Spacing",
                    valueLabel: "\(Int(lineSpacing))",
                    value: $lineSpacing,
                    range: 2...18,
                    step: 1
                )
                labeledSlider(
                    title: "Page Margins",
                    valueLabel: "\(Int(horizontalPadding)) px",
                    value: $horizontalPadding,
                    range: 30...140,
                    step: 2
                )
            }
        }
        .padding(18)
        .background(palette.card)
        .clipShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .stroke(palette.divider, lineWidth: 1)
        }
    }

    private func header(_ document: SemanticDocument) -> some View {
        let title = displayTitle(for: document)
        let authors = displayAuthors(for: document)

        return VStack(alignment: .leading, spacing: 10) {
            Text("Fluid Reading Edition")
                .font(.caption)
                .textCase(.uppercase)
                .tracking(1.6)
                .foregroundStyle(palette.subtle)

            Text(title ?? "Untitled")
                .font(.system(size: fontSize + 16, weight: .semibold, design: .serif))
                .fixedSize(horizontal: false, vertical: true)
                .foregroundStyle(palette.text)

            if !authors.isEmpty {
                Text(authors.joined(separator: ", "))
                    .font(.system(size: fontSize - 2, weight: .regular, design: .serif))
                    .foregroundStyle(palette.subtle)
            }
            if let abstractText = document.metadata.abstractText {
                VStack(alignment: .leading, spacing: 8) {
                    Text("Abstract")
                        .font(.system(size: fontSize, weight: .semibold, design: .serif))
                        .foregroundStyle(palette.text)
                    Text(abstractText)
                        .font(.system(size: fontSize, weight: .regular, design: .serif))
                        .lineSpacing(lineSpacing)
                        .foregroundStyle(palette.text.opacity(0.92))
                }
                .padding(22)
                .background(palette.card)
                .clipShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
                .overlay {
                    RoundedRectangle(cornerRadius: 16, style: .continuous)
                        .stroke(palette.divider, lineWidth: 1)
                }
            }
        }
        .padding(.bottom, 12)
    }

    private func labeledSlider(
        title: String,
        valueLabel: String,
        value: Binding<Double>,
        range: ClosedRange<Double>,
        step: Double
    ) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text(title)
                    .font(.caption)
                    .foregroundStyle(palette.subtle)
                Spacer()
                Text(valueLabel)
                    .font(.caption.monospacedDigit())
                    .foregroundStyle(palette.text)
            }

            Slider(value: value, in: range, step: step)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func sectionView(_ section: PresentedSection) -> some View {
        VStack(alignment: .leading, spacing: 18) {
            VStack(alignment: .leading, spacing: 6) {
                Text("Page \(section.pageStart)")
                    .font(.caption)
                    .foregroundStyle(palette.subtle)
                Text(section.title)
                    .font(.system(size: fontSize + 3 - Double(section.level), weight: .semibold, design: .serif))
                    .foregroundStyle(palette.text)
            }

            ForEach(section.blocks) { block in
                blockView(block)
                    .id(block.id)
            }
        }
        .padding(.top, 12)
    }

    @ViewBuilder
    private func blockView(_ block: Block) -> some View {
        switch block.kind {
        case .heading(let level):
            Text(block.text)
                .font(.system(size: fontSize + 4 - Double(level), weight: .semibold, design: .serif))
                .foregroundStyle(palette.text)
                .padding(.top, 8)
                .contextMenu { provenanceMenu(block) }
        case .list:
            Text("• \(block.text)")
                .font(.system(size: fontSize, weight: .regular, design: .serif))
                .lineSpacing(lineSpacing)
                .foregroundStyle(palette.text)
                .contextMenu { provenanceMenu(block) }
        case .quote:
            Text(block.text)
                .font(.system(size: fontSize, weight: .regular, design: .serif))
                .italic()
                .lineSpacing(lineSpacing)
                .foregroundStyle(palette.text)
                .padding(.leading, 18)
                .overlay(alignment: .leading) {
                    Rectangle().fill(palette.subtle.opacity(0.35)).frame(width: 2)
                }
                .contextMenu { provenanceMenu(block) }
        case .code:
            ScrollView(.horizontal, showsIndicators: false) {
                Text(block.text)
                    .font(.system(size: fontSize - 2, weight: .regular, design: .monospaced))
                    .foregroundStyle(palette.text)
                    .padding(16)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .background(palette.card)
                .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
                .contextMenu { provenanceMenu(block) }
        case .figure, .table:
            VStack(alignment: .leading, spacing: 8) {
                Text(block.text)
                    .font(.system(size: fontSize - 1, weight: .medium, design: .serif))
                    .foregroundStyle(palette.text)
                Text("Source page \(block.pageStart). Rendered as semantic placeholder with PDF fallback.")
                    .font(.caption)
                    .foregroundStyle(palette.subtle)
            }
            .padding(18)
            .background(palette.card)
            .clipShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: 14, style: .continuous)
                    .stroke(palette.divider, lineWidth: 1)
            }
            .contextMenu { provenanceMenu(block) }
        case .equation:
            Text(block.text)
                .font(.system(size: fontSize, weight: .regular, design: .serif))
                .foregroundStyle(palette.text)
                .frame(maxWidth: .infinity, alignment: .center)
                .padding(.vertical, 12)
                .contextMenu { provenanceMenu(block) }
        case .footnote:
            Text(block.text)
                .font(.system(size: max(12, fontSize - 4), weight: .regular, design: .serif))
                .lineSpacing(max(2, lineSpacing - 3))
                .foregroundStyle(palette.subtle)
                .contextMenu { provenanceMenu(block) }
        case .paragraph:
            paragraphView(block)
        }
    }

    private func paragraphView(_ block: Block) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(block.text)
                .font(.system(size: fontSize, weight: .regular, design: .serif))
                .lineSpacing(lineSpacing)
                .foregroundStyle(palette.text)
                .textSelection(.enabled)
            if block.confidence < 0.65 || !block.warnings.isEmpty {
                Text(block.warnings.first ?? "Low-confidence block; compare against source page.")
                    .font(.caption)
                    .foregroundStyle(palette.subtle)
            }
        }
        .padding(.vertical, 4)
        .contextMenu { provenanceMenu(block) }
    }

    private func displayTitle(for document: SemanticDocument) -> String? {
        if let title = document.metadata.title?.trimmingCharacters(in: .whitespacesAndNewlines),
           isUsefulDisplayTitle(title) {
            return title
        }

        return presentedSections(for: document)
            .first(where: { $0.title != "Front Matter" })?
            .title
    }

    private func displayAuthors(for document: SemanticDocument) -> [String] {
        document.metadata.authors.filter(isLikelyAuthorName)
    }

    private func presentedSections(for document: SemanticDocument) -> [PresentedSection] {
        let blocksByID = Dictionary(document.blocks.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
        return document.sections.compactMap { section in
            let blocks = section.blockIDs
                .compactMap { blocksByID[$0] }
                .filter { shouldPresent(block: $0, in: section, document: document) }

            guard !blocks.isEmpty else {
                return nil
            }

            return PresentedSection(
                id: section.id,
                title: presentedSectionTitle(section.title),
                level: section.level,
                pageStart: section.pageStart,
                blocks: blocks
            )
        }
    }

    private func shouldPresent(block: Block, in section: SemanticSection, document: SemanticDocument) -> Bool {
        let text = block.text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return false }

        let normalized = text.lowercased()
        let title = document.metadata.title?.lowercased()
        let authors = Set(displayAuthors(for: document).map { $0.lowercased() })

        if normalized == title || authors.contains(normalized) {
            return false
        }

        // The section header already shows its heading text.
        if case .heading = block.kind, text == section.title {
            return false
        }

        if normalized.contains("papers are available from the author") {
            return false
        }

        if case .footnote = block.kind {
            return false
        }

        if section.title == "Front Matter" {
            if case .paragraph = block.kind {
                if block.confidence < 0.82 || text.count < 48 || looksLikeVenueRollup(text) {
                    return false
                }
            }
        }

        if case .paragraph = block.kind, text.count < 24, block.confidence < 0.8 {
            return false
        }

        return true
    }

    private func presentedSectionTitle(_ title: String) -> String {
        title == "Front Matter" ? "Opening Material" : title
    }

    private func isUsefulDisplayTitle(_ text: String) -> Bool {
        let lower = text.lowercased()
        return !lower.contains("available from the author")
            && !lower.starts(with: "copyright")
            && text.split(separator: " ").count >= 4
    }

    private func isLikelyAuthorName(_ text: String) -> Bool {
        let tokens = text.split(separator: " ")
        guard (2...4).contains(tokens.count) else { return false }
        return tokens.allSatisfy { token in
            guard let first = token.first else { return false }
            return first.isUppercase
        }
    }

    private func looksLikeVenueRollup(_ text: String) -> Bool {
        text.filter { $0 == "," }.count >= 3 && text.split(separator: " ").count >= 8
    }

    private func provenanceMenu(_ block: Block) -> some View {
        Group {
            Button("Show Source Page \(block.pageStart)") {
                store.jumpToBlock(block.id)
            }
        }
    }

    private func pdfColumn(_ document: SemanticDocument) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Source PDF")
                .font(.headline)
                .foregroundStyle(palette.text)
            if let pdfURL = store.pdfURL {
                PDFPreview(pdfURL: pdfURL, pageIndex: selectedPage(document: document))
                    .clipShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
            } else {
                ContentUnavailableView("Open source PDF", systemImage: "doc.richtext", description: Text("Load the original PDF to compare the reflowed reading view against the source page."))
            }
        }
        .padding(22)
        .background(palette.pdfChrome)
    }

    private func selectedPage(document: SemanticDocument) -> Int {
        guard let selectedBlockID = store.selectedBlockID,
              let block = document.blocks.first(where: { $0.id == selectedBlockID }) else {
            return document.blocks.first?.pageStart ?? 1
        }
        return block.pageStart
    }
}
