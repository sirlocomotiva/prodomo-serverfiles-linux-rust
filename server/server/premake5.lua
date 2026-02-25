--[[
    About Premake5, see https://github.com/premake/premake-core/wiki
    Author: ProdomoFiles
]]

newoption {
    trigger = "with-warnings",
    description = "Controls the number of warnings that are shown by the compiler.",
}

workspace "ProdomoFiles"
    architecture "x86"
	
	if not _OPTIONS["with-warnings"] then
        warnings "off"
    else
        warnings "Default"
    end

    configurations{
        "Debug",
        "Release",
        "Fast-Release"
    }
    staticruntime "on"
    language "C++"
    cppdialect "C++17"
    largeaddressaware "on"
    outputdir = os.getcwd() .. "/bin"
    local bsd_flags ={
        "-msse2", "-mssse3", "-pipe", "-D_THREAD_SAFE",
        "-Wall", "-static", "-Wdeprecated-register", "-w",
    }
    local bsd_debug ={
        "-g3", "-O0", "-ggdb",
    }
    local bsd_release ={
        "-Ofast", "-g0", "-fexceptions",
    }
    includedirs{
        "../extern/include"
    }
    libdirs{
        "../extern/lib"
    }

    filter "configurations:Debug"
       defines { "DEBUG" }
       runtime "Debug"
       targetsuffix "d"
       symbols "full"

    filter "configurations:Release"
       defines { "NDEBUG" }
       runtime "Release"
       symbols "full"

    filter "configurations:Fast-Release"
       defines { "NDEBUG" }
       runtime "Release"
       optimize "speed"
    
    filter "action:vs*"
        systemversion "latest"
        defines{
            "_CRT_SECURE_NO_WARNINGS",
            "__WIN32__",
            "_USE_32BIT_TIME_T"
        }
        disablewarnings{
            "4307",
            "4996",
            "4244",
            "4267"
        }
        characterset "MBCS"

    configuration{"bsd", "debug"}
        buildoptions{bsd_debug, bsd_flags}
        linkoptions{bsd_debug, bsd_flags}
     
    configuration{"bsd", "release"}
        buildoptions{bsd_debug, bsd_flags}
        linkoptions{bsd_debug, bsd_flags}

    configuration{"bsd", "fast-release"}
        buildoptions{bsd_release, bsd_flags}
        linkoptions{bsd_release,bsd_flags}

    include "libgame"
    include "liblua"
    include "libpoly"
    include "libsql"
    include "libthecore"
    include "quest/qc.lua"
    include "db"
    include "game"
-- end of workspace
