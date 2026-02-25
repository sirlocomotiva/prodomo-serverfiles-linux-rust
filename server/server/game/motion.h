#ifndef __INC_GAME_MOTION_H__
#define __INC_GAME_MOTION_H__

#include "../common/d3dtype.h"
#include "../common/prodomodefines.h"

enum EMotionMode
{
	MOTION_MODE_GENERAL,
	MOTION_MODE_ONEHAND_SWORD,
	MOTION_MODE_TWOHAND_SWORD,
	MOTION_MODE_DUALHAND_SWORD,
	MOTION_MODE_BOW,
	MOTION_MODE_BELL,
	MOTION_MODE_FAN,
	MOTION_MODE_HORSE,
	MOTION_MODE_MAX_NUM
};

enum EPublicMotion
{
	MOTION_NONE,                // 0 ����
	MOTION_WAIT,                // 1 ���          (00.msa)
	MOTION_WALK,                // 2 �ȱ�          (02.msa)
	MOTION_RUN,                 // 3 �ٱ�          (03.msa)
	MOTION_CHANGE_WEAPON,       // 4 ����ٲٱ�
	MOTION_DAMAGE,              // 5 ����±�      (30.msa)
	MOTION_DAMAGE_FLYING,       // 6 ���鳯�ư���  (32.msa)
	MOTION_STAND_UP,            // 7 �����Ͼ��  (33.msa)
	MOTION_DAMAGE_BACK,         // 8 �ĸ�±�      (34.msa)
	MOTION_DAMAGE_FLYING_BACK,  // 9 �ĸ鳯�ư���  (35.msa)
	MOTION_STAND_UP_BACK,       // 10 �ĸ��Ͼ�� (26.msa)
	MOTION_DEAD,                // 11 �ױ�         (31.msa)
	MOTION_DEAD_BACK,           // 12 �ĸ��ױ�     (37.msa)
	MOTION_NORMAL_ATTACK,		// 13 �⺻ ����
	MOTION_COMBO_ATTACK_1,		// 14 �޺� ����
	MOTION_COMBO_ATTACK_2,      // 15 �޺� ����
	MOTION_COMBO_ATTACK_3,      // 16 �޺� ����
	MOTION_COMBO_ATTACK_4,      // 17 �޺� ����
	MOTION_COMBO_ATTACK_5,      // 18 �޺� ����
	MOTION_COMBO_ATTACK_6,      // 19 �޺� ����
	MOTION_COMBO_ATTACK_7,      // 20 �޺� ����
	MOTION_COMBO_ATTACK_8,      // 21 �޺� ����
	MOTION_INTRO_WAIT,          // 22 ����ȭ�� ���
	MOTION_INTRO_SELECTED,      // 23 ����ȭ�� ����
	MOTION_INTRO_NOT_SELECTED,  // 24 ����ȭ�� ����
	MOTION_SPAWN,               // 25 ��ȯ
	MOTION_FISHING_THROW,       // 26 ���� ������
	MOTION_FISHING_WAIT,        // 27 ���� ���
	MOTION_FISHING_STOP,        // 28 ���� �׸��α�
	MOTION_FISHING_REACT,       // 29 ���� ����
	MOTION_FISHING_CATCH,       // 30 ���� ���
	MOTION_FISHING_FAIL,        // 31 ���� ����
	MOTION_STOP,                // 32 �� ���߱�
	MOTION_SPECIAL_1,           // 33 ���� ��ų
	MOTION_SPECIAL_2,           // 34
	MOTION_SPECIAL_3,			// 35
	MOTION_SPECIAL_4,			// 36
	MOTION_SPECIAL_5,			// 37
	PUBLIC_MOTION_END,
	MOTION_MAX_NUM = PUBLIC_MOTION_END,
};

class CMob;

class CMotion
{
	public:
		CMotion();
		~CMotion();

		bool			LoadFromFile(const char * c_pszFileName);
		bool			LoadMobSkillFromFile(const char * c_pszFileName, CMob * pMob, int iSkillIndex);

		float			GetDuration() const;
		const D3DVECTOR &	GetAccumVector() const;

		bool			IsEmpty();

	protected:
		bool			m_isEmpty;
		float			m_fDuration;
		bool			m_isAccumulation;
		D3DVECTOR		m_vec3Accumulation;
};

typedef DWORD MOTION_KEY;

#define MAKE_MOTION_KEY(mode, index)			( ((mode) << 24) | ((index) << 8) | (0) )
#define MAKE_RANDOM_MOTION_KEY(mode, index, type)	( ((mode) << 24) | ((index) << 8) | (type) )

#define GET_MOTION_MODE(key)				((BYTE) ((key >> 24) & 0xFF))
#define GET_MOTION_INDEX(key)				((WORD) ((key >> 8) & 0xFFFF))
#define GET_MOTION_SUB_INDEX(key)			((BYTE) ((key) & 0xFF))

class CMotionSet
{
	public:
		typedef std::map<DWORD, CMotion *>	TContainer;
		typedef TContainer::iterator		iterator;
		typedef TContainer::const_iterator	const_iterator;

	public:
		CMotionSet();
		~CMotionSet();

		void		Insert(DWORD dwKey, CMotion * pkMotion);
		bool		Load(const char * szFileName, int mode, int motion);

		const CMotion *	GetMotion(DWORD dwKey) const;

	protected:
		// DWORD = MOTION_KEY
		TContainer			m_map_pkMotion;
};

class CMotionManager : public singleton<CMotionManager>
{
	public:
		typedef std::map<DWORD, CMotionSet *> TContainer;
		typedef TContainer::iterator iterator;

		CMotionManager();
		virtual ~CMotionManager();

		bool			Build();

		const CMotionSet *	GetMotionSet(DWORD dwVnum);
		const CMotion *		GetMotion(DWORD dwVnum, DWORD dwKey);
		float			    GetMotionDuration(DWORD dwVnum, DWORD dwKey);

		// POLYMORPH_BUG_FIX
		float			    GetNormalAttackDuration(DWORD dwVnum);
		// END_OF_POLYMORPH_BUG_FIX

	protected:
		// DWORD = JOB or MONSTER VNUM
		TContainer		m_map_pkMotionSet;

		// POLYMORPH_BUG_FIX
		std::map<DWORD, float> m_map_normalAttackDuration;
		// END_OF_POLYMORPH_BUG_FIX
};

#endif
