-- Simon Says Dictionary (reconstructed from VisualSource)
-- Holds the word list and answers script.Parent.Bindables.GatherWord with a random word.

local WordDictionary = {}

local words = {
	"Hello", "World", "Game", "Player", "Script", "Code", "Roblox", "Function", "Variable", "Object",
	"Loop", "Event", "Boolean", "String", "Table", "Index", "Instance", "Method", "Parameter", "Argument",
	"Return", "Data", "Debug", "Print", "Execute", "Spawn", "Destroy", "Clone", "Parent", "Number",
	"Integer", "Float", "Decimal", "Addition", "Subtraction", "Multiplication", "Division", "Modulo", "Exponent", "Factorial",
	"Algorithm", "Equation", "Probability", "Percentage", "Ratio", "Mean", "Median", "Mode", "Derivative", "Integral",
	"Prime", "Odd", "Even", "Sequence", "Series", "Root", "Matrix", "Vector", "Graph", "Logarithm",
	"Variable", "Constant", "Summation", "Infinity", "Physics", "Velocity", "Gravity", "Speed", "Force", "Vector",
	"Momentum", "Acceleration", "Mass", "Energy", "Friction", "Inertia", "Pressure", "Torque", "Power", "Wave",
	"Amplitude", "Frequency", "Vibration", "Impulse", "Trajectory", "Projectile", "Collision", "Magnetism", "Radiation", "Resonance",
	"Refraction", "Reflection", "Elasticity", "Viscosity", "Density", "Buoyancy", "Entropy", "Vortex", "CFrame", "Brick",
	"Part", "Mesh", "Anchor", "Collision", "Transparency", "Surface", "Texture", "Material", "Shader", "Render",
	"Polygon", "Vertex", "Normal", "Bevel", "Extrude", "Smooth", "Animation", "Rig", "Bone", "Keyframe",
	"Timeline", "PhysicsBody", "Gravity", "Pivot", "MassCenter", "Lighting", "Shadow", "Day", "Night", "Weather",
	"Fog", "Horizon", "Bloom", "Gradient", "Reflection", "Artificial", "Neural", "Algorithm", "DeepLearning", "MachineLearning",
	"Automation", "Dataset", "Model", "Training", "Inference", "Bias", "Supervised", "Unsupervised", "Pattern", "Recognition",
	"Hacking", "Firewall", "Encryption", "Authentication", "Token", "Exploit", "Malware", "Spyware", "Trojan", "Phishing",
	"Rootkit", "Backdoor", "Hashing", "Proxy", "DDoS", "Cyberattack", "Ransomware", "Server", "Client", "RemoteEvent",
	"RemoteFunction", "ReplicatedStorage", "DataStore", "Save", "Load", "Leaderboard", "Points", "Score", "Timer", "Countdown",
	"Checkpoint", "Teleport", "Packet", "Latency", "Ping", "Bandwidth", "Encryption", "Compression", "Cache", "Session",
	"Database", "Query", "Table", "Row", "Column", "Indexing", "Sorting", "Filtering", "Validation", "Map",
	"Spawn", "Character", "Animation", "Humanoid", "Move", "Jump", "Run", "Walk", "Swim", "Shoot",
	"Attack", "Health", "Damage", "Heal", "Power", "Energy", "Armor", "Shield", "Weapon", "Sword",
	"Gun", "Bow", "Arrow", "Magic", "Potion", "Treasure", "Coin", "Gem", "Key", "Door",
	"Mission", "Quest", "Challenge", "Enemy", "Boss", "Level", "Checkpoint", "Battle", "Strategy", "Stealth",
	"Defense", "Offense", "Ally", "Team", "Squad", "Victory", "Defeat", "Survival", "Stamina", "Strength",
	"Dexterity", "Agility", "Luck", "Critical", "Mana", "Potion", "Dagger", "Laser", "Spaceship", "Comet",
	"Wormhole", "Teleport", "Alien", "Galaxy", "BlackHole", "Nebula", "Cybernetic", "Quantum", "Nanobot", "Plasma",
	"Mech", "Android", "Cyberpunk", "Exosuit", "Hologram", "Dragon", "Elf", "Wizard", "Spell", "Mana",
	"Alchemy", "Crystal", "Summon", "Demon", "Knight", "Castle", "Runestone", "Grimoire", "Mystic", "Fairy",
	"Goblin", "Troll", "Orc", "Necromancer", "Stock", "Investment", "Market", "Bank", "Trade", "Cryptocurrency",
	"Blockchain", "Finance", "Economy", "Inflation", "Recession", "Interest", "Capital", "Profit", "Loss", "Revenue",
	"Entrepreneur", "Startup", "Negotiation", "Merger", "Acquisition", "Corporate", "Shareholder", "Biology", "Chemistry", "Physics",
	"Anatomy", "Botany", "Geology", "Ecology", "Genetics", "Molecule", "DNA", "Protein", "Evolution", "Photosynthesis",
	"Atom", "Electron", "Nucleus", "Fossil", "Climate", "Ecosystem", "Tundra", "Savanna", "Rainforest", "Coral",
	"Biodiversity", "Happy", "Sad", "Angry", "Excited", "Scared", "Nervous", "Brave", "Shy", "Confident",
	"Lonely", "Friendly", "Honest", "Kind", "Loyal", "Jealous", "Proud", "Calm", "Worried", "Cheerful",
	"Anxious", "Virtual", "Augmented", "Holographic", "Automation", "Cloud", "Database", "Cybernetics", "Satellite", "AI",
	"Robot", "Algorithm", "Interface", "Biometric", "Gesture", "Sensor", "Quantum", "Processor", "SSD", "Blockchain",
	"Cryptography", "Microchip", "Nanotechnology",
}

-- Copy every word into WordDictionary (the original did this with a loop)
for _, word in ipairs(words) do
	table.insert(WordDictionary, word)
end

-- Main script calls this to get a random word
script.Parent.Bindables.GatherWord.OnInvoke = function()
	local randomIndex = math.random(1, #WordDictionary)
	local word = WordDictionary[randomIndex]
	return word
end
