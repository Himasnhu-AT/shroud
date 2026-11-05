import unreal, os
OUT = os.environ["MH_TEX_OUT"]
paths = {
 "head_color": "/MetaHuman/IdentityTemplate/DefaultArchetypeTexture/T_MetaHumanIdentity_Head_D",
 "head_normal": "/MetaHuman/IdentityTemplate/DefaultArchetypeTexture/T_MetaHumanIdentity_Head_N",
 "teeth_color": "/MetaHuman/IdentityTemplate/DefaultArchetypeTexture/T_MetaHumanIdentity_Teeth_D",
 "eye_color": "/MetaHuman/IdentityTemplate/DefaultArchetypeTexture/T_MetaHumanIdentity_Eye_D",
 "iris": "/MetaHumanCharacter/Lookdev_UHM/Eye/Textures/T_Iris001_01_D",
 "sclera": "/MetaHumanCharacter/Lookdev_UHM/Eye/Textures/T_EyeSclera_D",
 "skin_micro_normal": "/MetaHumanCharacter/Lookdev_UHM/Skin/Textures/T_SkinMicroNormal",
 "body_color": "/MetaHumanCharacter/Textures/Shared/1K/T_Chr0000_Body_Underwear_BC",
 "body_normal": "/MetaHumanCharacter/Textures/Shared/1K/T_Chr0000_Body_Underwear_N",
}
for name, p in paths.items():
    tex = unreal.load_asset(p)
    if tex is None:
        unreal.log_warning("TEXEXPORT missing " + p); continue
    for ext in ("png", "tga"):
        t = unreal.AssetExportTask(); t.object = tex; t.filename = os.path.join(OUT, name + "." + ext)
        t.automated = True; t.prompt = False; t.replace_identical = True
        ok = unreal.Exporter.run_asset_export_task(t)
        if ok and os.path.exists(t.filename):
            unreal.log("TEXEXPORT ok %s %s %dx%d" % (name, ext, tex.blueprint_get_size_x(), tex.blueprint_get_size_y())); break
    else:
        unreal.log_warning("TEXEXPORT failed " + name)
