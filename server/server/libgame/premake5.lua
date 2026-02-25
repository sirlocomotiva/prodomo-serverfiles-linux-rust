project "libgame"
    kind "StaticLib"

    -- targetdir ("obj/%{cfg.buildcfg}")
    -- objdir ("obj/%{cfg.buildcfg}")

    files
    {
        "**.h",
        "**.cpp",
    }

    filter "action:vs*"
        pchheader "stdafx.h"
        pchsource "stdafx.cpp"

    filter "action:not vs*"
        pchheader "stdafx.h"