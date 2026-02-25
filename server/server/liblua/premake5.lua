project "liblua"
    kind "StaticLib"
    language "C"

    -- targetdir ("obj/%{cfg.buildcfg}")
    -- objdir ("obj/%{cfg.buildcfg}")

    files
    {
        "**.h",
        "**.c",
    }