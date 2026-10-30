# Ternary plots

A ternary chart shows samples of three components that share a whole:
alloy compositions, soil textures, phase diagrams, vote shares. Set
`Config.ternary` to `Some(TernaryOptions {})` and draw `DataRenderType::Ternary`
series; the data area holds an equilateral triangle whose corners are the
100 % of each component.
