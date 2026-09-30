// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "FluidPaperReader",
    platforms: [
        .macOS(.v14),
    ],
    products: [
        .executable(name: "FluidPaperReader", targets: ["FluidPaperReader"]),
    ],
    targets: [
        .executableTarget(
            name: "FluidPaperReader",
            path: "Sources"
        ),
    ]
)
