project "game"
    kind "ConsoleApp"

    targetdir (outputdir)
    objdir ("obj/%{cfg.buildcfg}")

    includedirs{
        "../liblua"
    }

    files
    {
        "**.cpp",
        "**.h",
        "**.inc",
    }

    links{
        "libgame",
        "libpoly",
        "libsql",
        "libthecore",
        "liblua",
        "lzo2"
    }

    filter "action:vs*"
        pchheader "stdafx.h"
        pchsource "stdafx.cpp"

    filter "action:not vs*"
        pchheader "stdafx.h"

    configuration{"release", "vs*"}
        linkoptions{"/SAFESEH:NO"}

    configuration{"debug", "vs*"}
        linkoptions{"/NODEFAULTLIB:libcmt"}
    
    filter "system:bsd"
        links{
            "pthread",
            "IL",
            "png",
            "tiff",
            "jpeg",
            "mng",
            "lcms",
            "jbig",
            "lzma",
            "md",
            "ssl",
            "crypto",
            "z",
			"mysqlclient"
        }
        defines{
            "__SVN_VERSION__=1337"
        }
    
    filter "system:windows"
        links{
            "ws2_32",
            "libmysql",
            "mysqlclient",
            "DevIL"
        }
