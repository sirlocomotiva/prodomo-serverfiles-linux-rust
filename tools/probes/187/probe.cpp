
template<int N> struct Size;
#pragma pack(1)
struct TwoLong { unsigned long a; unsigned long b; };
struct OneLong  { unsigned long a; };
struct OnePtr   { void* a; };
struct OneBool  { bool a; };
struct OneTimeT { long a; };
Size<sizeof(TwoLong)> c1;   // must be 8
Size<sizeof(OneLong)>  c2;   // must be 4
Size<sizeof(OnePtr)>   c3;   // must be 4
Size<sizeof(OneBool)>  c4;   // must be 1
Size<sizeof(OneTimeT)> c5;   // must be 4
