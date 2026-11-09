--[[
    MINECRAFT-STYLE TERRAIN GENERATOR
    Compatível com Roblox/Luau de aproximadamente mid-2015

    Características:
    - Mundo 500x500
    - Altura máxima 100
    - Água em Y = 50
    - Praias naturais
    - Planícies
    - Colinas
    - Montanhas
    - Pedra apenas em regiões altas
    - Neve nos picos
    - Árvores geradas proceduralmente
    - Chunks
    - Seed determinística
    - BrickColor
]]

local Workspace = game:GetService("Workspace")

local CONFIG = {
    -- Tamanho do mundo
    TerrainSize = Vector3.new(500, 100, 500),

    -- Tamanho dos blocos
    PartSize = Vector3.new(4, 1, 4),

    -- Tamanho dos chunks
    ChunkSize = Vector3.new(100, 100, 100),

    -- Altura máxima
    MaxHeight = 100,

    -- Altura média
    BaseHeight = 50,

    -- Nível da água
    WaterHeight = 50,

    -- Noise principal
    NoiseScale = 0.02,

    -- Noise das montanhas
    MountainScale = 0.006,

    -- Noise de detalhe
    DetailScale = 0.045,

    -- Intensidade das montanhas
    MountainStrength = 45,

    -- Pequenas variações
    DetailStrength = 5,

    -- Largura vertical das praias
    BeachHeight = 7,

    -- Onde começa a pedra
    StoneHeight = 67,

    -- Onde começa a neve
    SnowHeight = 88,

    -- Árvores
    TreeDensity = 0.025,
    MinimumTreeHeight = 53,
    TreeWaterDistance = 3,

    -- Seed
    Seed = 123456,

    -- Performance
    PartsPerYield = 250,

    -- Materiais
    GrassMaterial = Enum.Material.Grass,
    SandMaterial = Enum.Material.Sand,
    RockMaterial = Enum.Material.Rock,
    SnowMaterial = Enum.Material.Snow,
    WaterMaterial = Enum.Material.Water,
}

-- ============================================================
-- LIMPEZA
-- ============================================================

local oldTerrain =
    Workspace:FindFirstChild("GeneratedTerrain")

if oldTerrain then
    oldTerrain:Destroy()
end

-- ============================================================
-- PASTAS
-- ============================================================

local terrainFolder = Instance.new("Folder")
terrainFolder.Name = "GeneratedTerrain"
terrainFolder.Parent = Workspace

local groundFolder = Instance.new("Folder")
groundFolder.Name = "Ground"
groundFolder.Parent = terrainFolder

local waterFolder = Instance.new("Folder")
waterFolder.Name = "Water"
waterFolder.Parent = terrainFolder

local treesFolder = Instance.new("Folder")
treesFolder.Name = "Trees"
treesFolder.Parent = terrainFolder

-- ============================================================
-- RANDOM
-- ============================================================

local rng = Random.new(CONFIG.Seed)

-- ============================================================
-- TAMANHO DOS BLOCOS
-- ============================================================

local partSizeX = CONFIG.PartSize.X
local partSizeZ = CONFIG.PartSize.Z

local partCountX =
    math.floor(
        CONFIG.TerrainSize.X /
        partSizeX
    )

local partCountZ =
    math.floor(
        CONFIG.TerrainSize.Z /
        partSizeZ
    )

local chunkPartCountX =
    math.max(
        1,
        math.floor(
            CONFIG.ChunkSize.X /
            partSizeX
        )
    )

local chunkPartCountZ =
    math.max(
        1,
        math.floor(
            CONFIG.ChunkSize.Z /
            partSizeZ
        )
    )

local chunkCountX =
    math.ceil(
        partCountX /
        chunkPartCountX
    )

local chunkCountZ =
    math.ceil(
        partCountZ /
        chunkPartCountZ
    )

local worldOriginX =
    -CONFIG.TerrainSize.X / 2

local worldOriginZ =
    -CONFIG.TerrainSize.Z / 2

-- ============================================================
-- PERFORMANCE
-- ============================================================

local generatedParts = 0

local function yieldIfNeeded()
    generatedParts = generatedParts + 1

    if generatedParts %
        CONFIG.PartsPerYield == 0 then
        wait()
    end
end

-- ============================================================
-- NOISE
-- ============================================================

local noiseOffsetX =
    CONFIG.Seed * 0.00137

local noiseOffsetZ =
    CONFIG.Seed * 0.00241

-- ============================================================
-- ALTURA
-- ============================================================

local function getHeight(worldX, worldZ)
    local continentalNoise =
        math.noise(
            worldX * CONFIG.NoiseScale +
                noiseOffsetX,

            worldZ * CONFIG.NoiseScale +
                noiseOffsetZ
        )

    local mountainNoise =
        math.noise(
            worldX * CONFIG.MountainScale +
                noiseOffsetX + 300,

            worldZ * CONFIG.MountainScale +
                noiseOffsetZ + 300
        )

    local detailNoise =
        math.noise(
            worldX * CONFIG.DetailScale +
                noiseOffsetX + 600,

            worldZ * CONFIG.DetailScale +
                noiseOffsetZ + 600
        )

    local continental =
        (continentalNoise + 1) / 2

    local mountainMask =
        (continental - 0.48) / 0.52

    if mountainMask < 0 then
        mountainMask = 0
    end

    if mountainMask > 1 then
        mountainMask = 1
    end

    mountainMask =
        mountainMask * mountainMask

    local height = CONFIG.BaseHeight

    height =
        height +
        continentalNoise * 16

    height =
        height +
        mountainNoise *
        CONFIG.MountainStrength *
        mountainMask

    height =
        height +
        detailNoise *
        CONFIG.DetailStrength

    if height < 2 then
        height = 2
    end

    if height > CONFIG.MaxHeight then
        height = CONFIG.MaxHeight
    end

    return height
end

-- ============================================================
-- TEMPERATURA
-- ============================================================

local function getTemperature(worldX, worldZ)
    local temperatureNoise =
        math.noise(
            worldX * 0.008 +
                noiseOffsetX + 1000,

            worldZ * 0.008 +
                noiseOffsetZ + 1000
        )

    return (temperatureNoise + 1) / 2
end

-- ============================================================
-- MATERIAL E BRICKCOLOR
-- ============================================================

local function getMaterial(height, temperature)
    -- Praia

    local distanceFromWater =
        math.abs(
            height -
            CONFIG.WaterHeight
        )

    if distanceFromWater <= CONFIG.BeachHeight then
        return
            CONFIG.SandMaterial,
            BrickColor.new("Brick yellow")
    end

    -- Neve

    if height >= CONFIG.SnowHeight then
        return
            CONFIG.SnowMaterial,
            BrickColor.new("Institutional white")
    end

    -- Pedra

    if height >= CONFIG.StoneHeight then
        if temperature < 0.25 then
            return
                CONFIG.RockMaterial,
                BrickColor.new("Medium stone grey")
        end

        return
            CONFIG.RockMaterial,
            BrickColor.new("Dark stone grey")
    end

    -- Grama

    if temperature < 0.2 then
        return
            CONFIG.GrassMaterial,
            BrickColor.new("Earth green")
    end

    return
        CONFIG.GrassMaterial,
        BrickColor.new("Bright green")
end

-- ============================================================
-- BLOCO DO CHÃO
-- ============================================================

local function createGroundBlock(
    worldX,
    worldZ,
    height,
    material,
    brickColor
)
    local block = Instance.new("Part")

    block.Name = "GroundBlock"
    block.Anchored = true
    block.CanCollide = true
    block.Material = material
    block.BrickColor = brickColor

    block.Size =
        Vector3.new(
            CONFIG.PartSize.X,
            height,
            CONFIG.PartSize.Z
        )

    block.Position =
        Vector3.new(
            worldX,
            height / 2,
            worldZ
        )

    block.Parent = groundFolder

    return block
end

-- ============================================================
-- ÁGUA
-- ============================================================

local function createWaterBlock(worldX, worldZ)
    local water = Instance.new("Part")

    water.Name = "WaterBlock"
    water.Anchored = true
    water.CanCollide = false
    water.Material = CONFIG.WaterMaterial
    water.Transparency = 0.35
    water.BrickColor = BrickColor.new("Bright blue")

    water.Size =
        Vector3.new(
            CONFIG.PartSize.X,
            CONFIG.WaterHeight,
            CONFIG.PartSize.Z
        )

    water.Position =
        Vector3.new(
            worldX,
            CONFIG.WaterHeight / 2,
            worldZ
        )

    water.Parent = waterFolder

    return water
end

-- ============================================================
-- ÁRVORE
-- ============================================================

local function createTree(worldX, worldZ, height)
    -- Não cria árvore dentro da água.

    if height <=
        CONFIG.WaterHeight +
        CONFIG.TreeWaterDistance then
        return
    end

    -- Não cria árvores em montanhas.

    if height >= CONFIG.StoneHeight then
        return
    end

    -- Altura mínima.

    if height < CONFIG.MinimumTreeHeight then
        return
    end

    -- Densidade.

    if rng:NextNumber() > CONFIG.TreeDensity then
        return
    end

    -- Pasta individual da árvore.

    local tree = Instance.new("Model")
    tree.Name = "Tree"

    -- TAMANHO DA ÁRVORE

    local trunkHeight =
        rng:NextInteger(10, 15)

    local trunkWidth = 2

    -- TRONCO

    local trunk = Instance.new("Part")

    trunk.Name = "Trunk"
    trunk.Anchored = true
    trunk.CanCollide = true
    trunk.Material = Enum.Material.Wood
    trunk.BrickColor = BrickColor.new("Reddish brown")

    trunk.Size =
        Vector3.new(
            trunkWidth,
            trunkHeight,
            trunkWidth
        )

    trunk.Position =
        Vector3.new(
            worldX,
            height + trunkHeight / 2,
            worldZ
        )

    trunk.Parent = tree

    -- COPA

    local leavesColor =
        BrickColor.new("Earth green")

    local leafSize = 4

    local leafBaseY =
        height + trunkHeight

    -- Folhas centrais.

    local leafOffsets = {
        Vector3.new(0, 2, 0),
        Vector3.new(0, 5, 0),
        Vector3.new(0, 8, 0),
        Vector3.new(4, 4, 0),
        Vector3.new(-4, 4, 0),
        Vector3.new(0, 4, 4),
        Vector3.new(0, 4, -4),
        Vector3.new(4, 7, 0),
        Vector3.new(-4, 7, 0),
        Vector3.new(0, 7, 4),
        Vector3.new(0, 7, -4),
        Vector3.new(0, 10, 0),
    }

    local i

    for i = 1, #leafOffsets do
        local offset = leafOffsets[i]

        local leaves = Instance.new("Part")

        leaves.Name = "Leaves"
        leaves.Anchored = true
        leaves.CanCollide = false
        leaves.Material = Enum.Material.Grass
        leaves.BrickColor = leavesColor

        leaves.Size =
            Vector3.new(
                leafSize,
                leafSize,
                leafSize
            )

        leaves.Position =
            Vector3.new(
                worldX + offset.X,
                leafBaseY + offset.Y,
                worldZ + offset.Z
            )

        leaves.Parent = tree
    end

    -- VARIAÇÃO NA ÁRVORE

    local side =
        rng:NextInteger(0, 3)

    local branchOffset

    if side == 0 then
        branchOffset = Vector3.new(4, 6, 0)
    elseif side == 1 then
        branchOffset = Vector3.new(-4, 6, 0)
    elseif side == 2 then
        branchOffset = Vector3.new(0, 6, 4)
    else
        branchOffset = Vector3.new(0, 6, -4)
    end

    local extraLeaves = Instance.new("Part")

    extraLeaves.Name = "Leaves"
    extraLeaves.Anchored = true
    extraLeaves.CanCollide = false
    extraLeaves.Material = Enum.Material.Grass
    extraLeaves.BrickColor = leavesColor

    extraLeaves.Size =
        Vector3.new(4, 4, 4)

    extraLeaves.Position =
        Vector3.new(
            worldX + branchOffset.X,
            leafBaseY + branchOffset.Y,
            worldZ + branchOffset.Z
        )

    extraLeaves.Parent = tree
    tree.Parent = treesFolder
end

-- ============================================================
-- CHUNK
-- ============================================================

local function generateChunk(chunkX, chunkZ)
    local startX =
        chunkX * chunkPartCountX

    local startZ =
        chunkZ * chunkPartCountZ

    local endX =
        math.min(
            startX + chunkPartCountX - 1,
            partCountX - 1
        )

    local endZ =
        math.min(
            startZ + chunkPartCountZ - 1,
            partCountZ - 1
        )

    for globalX = startX, endX do
        for globalZ = startZ, endZ do
            -- Coordenadas físicas.

            local worldX =
                worldOriginX +
                globalX * partSizeX +
                partSizeX / 2

            local worldZ =
                worldOriginZ +
                globalZ * partSizeZ +
                partSizeZ / 2

            -- Altura.

            local height =
                getHeight(worldX, worldZ)

            -- Temperatura.

            local temperature =
                getTemperature(worldX, worldZ)

            -- Material e cor.

            local material, brickColor =
                getMaterial(height, temperature)

            -- Chão.

            createGroundBlock(
                worldX,
                worldZ,
                height,
                material,
                brickColor
            )

            -- Água.

            if height < CONFIG.WaterHeight then
                createWaterBlock(worldX, worldZ)
            end

            -- Árvore.

            createTree(worldX, worldZ, height)

            yieldIfNeeded()
        end
    end
end

-- ============================================================
-- GERAÇÃO
-- ============================================================

print("Iniciando geração do terreno...")

print(
    "Chunks:",
    chunkCountX,
    "x",
    chunkCountZ
)

local chunkX
local chunkZ

for chunkX = 0, chunkCountX - 1 do
    for chunkZ = 0, chunkCountZ - 1 do
        generateChunk(chunkX, chunkZ)

        -- Compatibilidade: sem string.format.

        print(
            "Chunk gerado:",
            chunkX,
            chunkZ
        )

        wait()
    end
end

print("Terreno gerado com sucesso!")