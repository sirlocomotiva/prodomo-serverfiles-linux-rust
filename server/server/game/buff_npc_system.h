class CHARACTER;

#ifdef ENABLE_ASLAN_BUFF_NPC_SYSTEM
struct SBuffNPCAbility
{
};

class CBuffNPCActor
{
	public:
		enum EBuffNPCOptions
		{
			EBuffNPCOption_Followable		= 1 << 0,
			EBuffNPCOption_Mountable		= 1 << 1,
			EBuffNPCOption_Summonable		= 1 << 2,
			EBuffNPCOption_Combatable		= 1 << 3,		
		};

		CBuffNPCActor(LPCHARACTER owner, DWORD options = EBuffNPCOption_Followable | EBuffNPCOption_Summonable);
		virtual ~CBuffNPCActor();

		virtual bool	Update(DWORD deltaTime);
		virtual bool	_UpdateFollowAI();
		virtual bool	_UpdatAloneActionAI();

	private:
		bool Follow(float fMinDistance = 150.f);

	public:
		void		Destroy();
		
		LPCHARACTER		GetCharacter()	const					{ return m_pkChar; }
		LPCHARACTER		GetOwner()	const						{ return m_pkOwner; }
		DWORD			GetVID() const							{ return m_dwVID; }
		DWORD			GetVnum() const							{ return m_dwVnum; }

		bool				HasOption(EBuffNPCOptions option) const		{ return m_dwOptions & option; }

		void				SetSkillUseStatus(int skillSlot, int value);

		void				SetName(const char* buffName);
		void				SetCMDLevel(DWORD level);
		void				SetLevel(DWORD level);
		void				SetLevelUp();
		void				SetInt(DWORD value);

		void				LoadBuffNPC();
		void				ClearBuffNPC();

		DWORD			Summon();
		void				Unsummon();
		void				UpdateBuffEquipment();

		bool				IsSummoned() const			{ return 0 != m_pkChar; }
		
		void				SetLastSkillTime(DWORD time)	{ m_dwLastSkillTime = time; }
		DWORD			GetLastSkillTime()	{ return m_dwLastSkillTime; }
		
		void				SetIsActive(bool isActive)	{ m_bIsActive = isActive; }
		DWORD			IsActive()	{ return m_bIsActive; }

		DWORD			GetLevel() { return m_dwLevel; }
		DWORD			GetSex() { return m_dwSex; }
		
		void				SetExp(DWORD exp);
		DWORD			GetExp() { return m_dwEXP; }
		
		void				SetNextExp(int nextExp);
		int					GetNextExp() { return m_dwNextEXP; }

		int					GetLevelStep() { return m_dwLevelStep; }

		int					GetSkillID(int iSkillVnum);
		int					GetSkillGrade(int iSkillLevel);

		void				SetClearSkill(DWORD dwSkillVnum);
		void				SetChangeSex();

		void				SetSkill(DWORD dwSkillSlot, DWORD dwSkillLevel);
		void				LearnSkillByPoint(DWORD dwSkillType);
		void				LearnSkillByBook(DWORD dwSkillVnum);
		void				LearnSkillBySoulstone(DWORD dwSkillVnum);
		void				UpdateTime();
		void				SaveSQL();
		bool				UseSkill();

	private:
		LPCHARACTER		m_pkChar;
		LPCHARACTER		m_pkOwner;

		short					m_originalMoveSpeed;

		bool					m_bIsActive;

		DWORD				m_dwVnum;
		DWORD				m_dwVID;
		DWORD				m_dwOptions;
		DWORD				m_dwLastSkillTime;
		DWORD				m_dwLastActionTime;
			
		std::string			m_name;
		DWORD				m_dwSex;
		DWORD				m_dwLevel;	
		DWORD				m_dwLevelStep; 
		DWORD				m_dwEXP;
		DWORD				m_dwNextEXP;
		DWORD				m_dwSkillPoints;
		DWORD				m_dwIntValue;
		DWORD				m_dwSkill[6];
		DWORD				m_dwSkillNextUse[6];
		DWORD				m_dwSkillInUse[6];

		DWORD				m_dwUpdatePeriod;
		DWORD				m_dwLastUpdateTime;
		LPEVENT			m_pkBuffNPCSystemExpireEvent;
};

#endif
