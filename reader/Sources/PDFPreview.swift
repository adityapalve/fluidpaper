import PDFKit
import SwiftUI

struct PDFPreview: NSViewRepresentable {
    let pdfURL: URL
    let pageIndex: Int

    func makeNSView(context: Context) -> PDFView {
        let view = PDFView()
        view.autoScales = true
        view.displayMode = .singlePageContinuous
        view.displaysPageBreaks = true
        view.backgroundColor = .windowBackgroundColor
        return view
    }

    func updateNSView(_ view: PDFView, context: Context) {
        if view.document?.documentURL != pdfURL {
            view.document = PDFDocument(url: pdfURL)
        }

        guard let document = view.document, pageIndex > 0, let page = document.page(at: pageIndex - 1) else {
            return
        }

        view.go(to: page)
    }
}
