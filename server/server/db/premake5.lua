project "db"
    kind "ConsoleApp"

    targetdir (outputdir)
    objdir ("obj/%{cfg.buildcfg}")

    files
    {
        "**.cpp",
        "**.h",
    }

    links{
        "libgame",
        "libpoly",
        "libsql",
        "libthecore"
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
            "mysqlclient",
            "ssl",
            "crypto",
            "z"
        }
        defines{
            "__SVN_VERSION__=1337"
        }
    
    filter "system:windows"
        links{
            "ws2_32",
            "libmysql",
            "mysqlclient"
        }
