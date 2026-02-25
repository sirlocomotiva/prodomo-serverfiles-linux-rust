project "qc"
    kind "ConsoleApp"

    targetdir (outputdir)
    objdir ("obj/%{cfg.buildcfg}")

    includedirs{
        "../liblua"
    }

    files
    {
        "crc32.cpp",
        "qc.cpp",
        "stdafx.cpp",
        "stdafx.h",
        "crc32.h",
        "qc.h",
    }
    links{
        "liblua"
    }
    filter "action:vs*"
        pchheader "stdafx.h"
        pchsource "stdafx.cpp"

    filter "action:not vs*"
        pchheader "stdafx.h"

    configuration{"release", "vs*"}
        linkoptions{"/SAFESEH:NO"}

    filter "system:bsd"
        links{"pthread"}

project "qc_conversion"
    kind "ConsoleApp"

    targetdir (outputdir)
    objdir ("obj/%{cfg.buildcfg}")

    includedirs{
        "../liblua"
    }

    files
    {
        "crc32.cpp",
        "create_conversion.cpp",
        "stdafx.cpp",
        "stdafx.h",
        "crc32.h",
        "create_conversion.h",
    }
    links{
        "liblua",
        "libthecore"
    }
    filter "action:vs*"
        pchheader "stdafx.h"
        pchsource "stdafx.cpp"

    filter "action:not vs*"
        pchheader "stdafx.h"

    configuration{"release", "vs*"}
        linkoptions{"/SAFESEH:NO"}

    filter "system:bsd"
        links{"pthread"}

