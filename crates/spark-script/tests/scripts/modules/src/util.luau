local util = {}

function util.add(a: number, b: number): number
    return a + b
end

function util.spawn_cube()
    return spawn(Mesh.cube(), "orange")
end

return util
