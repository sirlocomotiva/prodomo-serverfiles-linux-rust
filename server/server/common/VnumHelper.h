#ifndef __HEADER_VNUM_HELPER__
#define	__HEADER_VNUM_HELPER__
#include "prodomodefines.h"

/**
	이미 존재하거나 앞으로 추가될 아이템, 몹 등을 소스에서 식별할 때 현재는 모두
	식별자(숫자=VNum)를 하드코딩하는 방식으로 되어있어서 가독성이 매우 떨어지는데

	앞으로는 소스만 봐도 어떤 아이템(혹은 몹)인지 알 수 있게 하자는 승철님의 제안으로 추가.

		* 이 파일은 변경이 잦을것으로 예상되는데 PCH에 넣으면 바뀔 때마다 전체 컴파일 해야하니
		일단은 필요한 cpp파일에서 include 해서 쓰도록 했음.

		* cpp에서 구현하면 컴파일 ~ 링크해야하니 그냥 common에 헤더만 넣었음. (game, db프로젝트 둘 다 사용 예정)

	@date	2011. 8. 29.
*/


class CItemVnumHelper
{
public:

#if defined(__ATTR_6TH_7TH__)
	static const DWORD GetAttr67MaterialVnumByLevel(int iLevel)
	{
		if (iLevel <= 29)
			return 39070; // 회색파편 ( 1 ~ 29 )
		else if (iLevel == 30)
			return 39071; // 흰색파편 ( 30 )
		else if (iLevel >= 31 && iLevel <= 39)
			return 39072; // 녹색파편 ( 31 ~ 39 )
		else if (iLevel >= 40 && iLevel <= 49)
			return 39073; // 노란파편 ( 40 ~ 49 )
		else if (iLevel >= 50 && iLevel <= 59)
			return 39074; // 파란파편 ( 50 ~ 59 )
		else if (iLevel >= 60 && iLevel <= 74)
			return 39075; // 보라파편 ( 60 ~ 74 )
		else if (iLevel == 75)
			return 39076; // 빨강파편 ( 75 )
		else if (iLevel >= 76 && iLevel <= 89)
			return 39077; // 무지개파편 ( 76 ~ 89 )
		else if (iLevel >= 90 && iLevel <= 104)
			return 39078; // 빛나는 회색파편 ( 90 ~ 104 )
		else if (iLevel == 105)
			return 39079; // 빛나는 녹색파편 ( 105 )
		else if (iLevel >= 106 && iLevel <= 120)
			return 39080; // 빛나는 노란파편 ( 106 ~ 120 )
		else if (iLevel >= 120 && iLevel <= 150)
			return 39081; // 성스러운파편(120) ( 120 ~ 150 )
		else
			return 0;
	}
#endif

	/// 독일 DVD용 불사조 소환권
	static	const bool	IsPhoenix(DWORD vnum)				{ return 53001 == vnum; }		// NOTE: 불사조 소환 아이템은 53001 이지만 mob-vnum은 34001 입니다.

	/// 라마단 이벤트 초승달의 반지 (원래는 라마단 이벤트용 특수 아이템이었으나 앞으로 여러 방향으로 재활용해서 계속 쓴다고 함)
	static	const bool	IsRamadanMoonRing(DWORD vnum)		{ return 71135 == vnum; }

	/// 할로윈 사탕 (스펙은 초승달의 반지와 동일)
	static	const bool	IsHalloweenCandy(DWORD vnum)		{ return 71136 == vnum; }

	/// 크리스마스 행복의 반지
	static	const bool	IsHappinessRing(DWORD vnum)		{ return 71143 == vnum; }

	/// 발렌타인 사랑의 팬던트
	static	const bool	IsLovePendant(DWORD vnum)		{ return 71145 == vnum; }
	
#ifdef ENABLE_AFFECT_RENEWAL
	/// Extended Blend
	static const bool IsExtendedBlend(DWORD vnum)
	{
		switch (vnum)
		{
		// INFINITE_DEWS
		case 20209:
		case 20210:
		case 20211:
		case 20212:
		case 20213:
		case 20214:
		case 20215:
		case 20216: // Energy Cristal
		// END_OF_INFINITE_DEWS

		// DRAGON_GOD_MEDALS
		case 20205:
		case 20206:
		case 20207:
		case 20208:
		// END_OF_DRAGON_GOD_MEDALS

		// CRITICAL_AND_PENETRATION
		case 20203:
		case 20204:
		// END_OF_CRITICAL_AND_PENETRATION

		// ATTACK_AND_MOVE_SPEED
		case 20201:
		case 20202:
		case 20217:
		// END_OF_ATTACK_AND_MOVE_SPEED
			return true;
		
		default:
			return false;
		}
	}
#endif
};

class CMobVnumHelper
{
public:
	/// 독일 DVD용 불사조 몹 번호
	static	bool	IsPhoenix(DWORD vnum)				{ return 34001 == vnum; }
	static	bool	IsIcePhoenix(DWORD vnum)				{ return 34003 == vnum; }
	/// PetSystem이 관리하는 펫인가?
	static	bool	IsPetUsingPetSystem(DWORD vnum)	{ return (IsPhoenix(vnum) || IsReindeerYoung(vnum)) || IsIcePhoenix(vnum); }

	/// 2011년 크리스마스 이벤트용 펫 (아기 순록)
	static	bool	IsReindeerYoung(DWORD vnum)	{ return 34002 == vnum; }

	/// 라마단 이벤트 보상용 흑마(20119) .. 할로윈 이벤트용 라마단 흑마 클론(스펙은 같음, 20219)
	static	bool	IsRamadanBlackHorse(DWORD vnum)		{ return 20119 == vnum || 20219 == vnum || 22022 == vnum; }
};

class CVnumHelper
{
};


#endif	//__HEADER_VNUM_HELPER__
