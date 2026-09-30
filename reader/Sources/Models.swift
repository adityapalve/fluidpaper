import Foundation

struct SemanticDocument: Decodable {
    let version: String
    let metadata: DocumentMetadata
    let sections: [SemanticSection]
    let blocks: [Block]
    let assets: [Asset]
    let pageMap: [PageMap]
    let diagnostics: Diagnostics

    private enum CodingKeys: String, CodingKey {
        case version
        case metadata
        case sections
        case blocks
        case assets
        case pageMap = "page_map"
        case diagnostics
    }
}

struct DocumentMetadata: Decodable {
    let title: String?
    let authors: [String]
    let abstractText: String?
    let pageCount: Int
    let sourcePath: String?

    private enum CodingKeys: String, CodingKey {
        case title
        case authors
        case abstractText = "abstract_text"
        case pageCount = "page_count"
        case sourcePath = "source_path"
    }
}

struct SemanticSection: Decodable, Identifiable {
    let id: String
    let title: String
    let level: Int
    let blockIDs: [String]
    let pageStart: Int

    private enum CodingKeys: String, CodingKey {
        case id
        case title
        case level
        case blockIDs = "block_ids"
        case pageStart = "page_start"
    }
}

struct Block: Decodable, Identifiable {
    let id: String
    let kind: BlockKind
    let text: String
    let spans: [InlineSpan]
    let pageStart: Int
    let pageEnd: Int
    let bboxRefs: [RegionRef]
    let confidence: Double
    let warnings: [String]

    private enum CodingKeys: String, CodingKey {
        case id
        case kind
        case text
        case spans
        case pageStart = "page_start"
        case pageEnd = "page_end"
        case bboxRefs = "bbox_refs"
        case confidence
        case warnings
    }
}

enum BlockKind: Decodable {
    case heading(level: Int)
    case paragraph
    case list
    case quote
    case code
    case figure(assetID: String?)
    case table(assetID: String?)
    case equation
    case footnote

    private enum CodingKeys: String, CodingKey {
        case type
        case data
        case level
        case assetID = "asset_id"
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let kind = try container.decode(String.self, forKey: .type)
        switch kind {
        case "heading":
            let nested = try container.nestedContainer(keyedBy: CodingKeys.self, forKey: .data)
            self = .heading(level: try nested.decode(Int.self, forKey: .level))
        case "figure":
            let nested = try container.nestedContainer(keyedBy: CodingKeys.self, forKey: .data)
            self = .figure(assetID: try nested.decodeIfPresent(String.self, forKey: .assetID))
        case "table":
            let nested = try container.nestedContainer(keyedBy: CodingKeys.self, forKey: .data)
            self = .table(assetID: try nested.decodeIfPresent(String.self, forKey: .assetID))
        case "paragraph":
            self = .paragraph
        case "list":
            self = .list
        case "quote":
            self = .quote
        case "code":
            self = .code
        case "equation":
            self = .equation
        case "footnote":
            self = .footnote
        default:
            throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath, debugDescription: "Unsupported block kind"))
        }
    }
}

struct InlineSpan: Decodable {
    let text: String
    let style: String
    let sourcePage: Int
    let bboxRefs: [RegionRef]

    private enum CodingKeys: String, CodingKey {
        case text
        case style
        case sourcePage = "source_page"
        case bboxRefs = "bbox_refs"
    }
}

struct Asset: Decodable, Identifiable {
    let id: String
    let kind: String
    let sourcePage: Int
    let bbox: BBox
    let caption: String?
    let confidence: Double

    private enum CodingKeys: String, CodingKey {
        case id
        case kind
        case sourcePage = "source_page"
        case bbox
        case caption
        case confidence
    }
}

struct PageMap: Decodable, Identifiable {
    let page: Int
    let width: Double
    let height: Double
    let blockIDs: [String]
    let regions: [PageRegion]

    var id: Int { page }

    private enum CodingKeys: String, CodingKey {
        case page
        case width
        case height
        case blockIDs = "block_ids"
        case regions
    }
}

struct PageRegion: Decodable {
    let kind: String
    let bbox: BBox
    let blockID: String?
    let confidence: Double

    private enum CodingKeys: String, CodingKey {
        case kind
        case bbox
        case blockID = "block_id"
        case confidence
    }
}

struct RegionRef: Decodable {
    let page: Int
    let bbox: BBox
}

struct BBox: Decodable {
    let x0: Double
    let y0: Double
    let x1: Double
    let y1: Double
}

struct Diagnostics: Decodable {
    let warnings: [String]
    let unsupportedRegions: [UnsupportedRegion]

    private enum CodingKeys: String, CodingKey {
        case warnings
        case unsupportedRegions = "unsupported_regions"
    }
}

struct UnsupportedRegion: Decodable, Identifiable {
    let page: Int
    let bbox: BBox
    let reason: String

    var id: String { "\(page)-\(reason)-\(bbox.x0)-\(bbox.y0)" }
}
