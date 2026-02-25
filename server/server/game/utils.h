
#ifndef __INC_METIN_II_UTILS_H__
#define __INC_METIN_II_UTILS_H__

#include <math.h>

#define IS_SET(flag, bit)		((flag) & (bit))
#define SET_BIT(var, bit)		((var) |= (bit))
#define REMOVE_BIT(var, bit)	((var) &= ~(bit))
#define TOGGLE_BIT(var, bit)	((var) = (var) ^ (bit))

inline float DISTANCE_SQRT(long dx, long dy)
{
    return ::sqrt((float)dx * dx + (float)dy * dy);
}

inline int DISTANCE_APPROX(int dx, int dy)
{
	int min, max;

	if (dx < 0)
		dx = -dx;

	if (dy < 0)
		dy = -dy;

	if (dx < dy)
	{
		min = dx;
		max = dy;
	}
	else
	{
		min = dy;
		max = dx;
	}

    // coefficients equivalent to ( 123/128 * max ) and ( 51/128 * min )
    return ((( max << 8 ) + ( max << 3 ) - ( max << 4 ) - ( max << 1 ) +
		( min << 7 ) - ( min << 5 ) + ( min << 3 ) - ( min << 1 )) >> 8 );
}


inline int getRemainingSecondsUntilMidnight(int unixTime) {
	// Get current time
	time_t now = time(0);

	// Convert the current time to tm struct
	tm* currentTime = localtime(&now);

	// Set the tm struct to the beginning of the next day (00:00)
	currentTime->tm_sec = 0;
	currentTime->tm_min = 0;
	currentTime->tm_hour = 0;
	currentTime->tm_mday += 1;

	// Calculate the difference in seconds between the given time and the beginning of the next day
	time_t nextMidnight = mktime(currentTime);
	int remainingSeconds = static_cast<int>(nextMidnight - unixTime);

	return remainingSeconds;
}

// Function to calculate remaining seconds until the end of the week (Sunday 23:59:59)
inline int getRemainingSecondsUntilEndOfWeek(int unixTime) {
	// Get current time
	time_t now = time(0);

	// Convert the current time to tm struct
	tm* currentTime = localtime(&now);

	// Calculate the number of days remaining until the end of the week (Sunday)
	int daysRemaining = 7 - currentTime->tm_wday;

	// Set the tm struct to the end of the week (Sunday 23:59:59)
	currentTime->tm_sec = 59;
	currentTime->tm_min = 59;
	currentTime->tm_hour = 23;
	currentTime->tm_mday += daysRemaining;

	// Calculate the difference in seconds between the given time and the end of the week
	time_t endOfWeek = mktime(currentTime);
	int remainingSeconds = static_cast<int>(endOfWeek - unixTime);

	return remainingSeconds;
}

#ifndef __WIN32__
inline WORD MAKEWORD(BYTE a, BYTE b)
{
	return static_cast<WORD>(a) | (static_cast<WORD>(b) << 8);
}
#endif

extern void set_global_time(time_t t);
extern time_t get_global_time();

#ifdef ENABLE_GAYA_SYSTEM
extern time_t init_gayaTime();
#endif

#include <string>
std::string mysql_hash_password(const char* tmp_pwd);

extern int	dice(int number, int size);
extern size_t str_lower(const char * src, char * dest, size_t dest_size);

extern void	skip_spaces(char **string);

extern const char *	one_argument(const char *argument, char *first_arg, size_t first_size);
extern const char *	two_arguments(const char *argument, char *first_arg, size_t first_size, char *second_arg, size_t second_size);
extern const char *	three_arguments(const char *argument, char *first_arg, size_t first_size, char *second_arg, size_t second_size, char *third_arg, size_t third_size);
extern const char* four_arguments(const char* argument, char* first_arg, size_t first_size, char* second_arg, size_t second_size, char* third_flag, size_t third_size, char* four_flag, size_t four_size);
extern const char *	first_cmd(const char *argument, char *first_arg, size_t first_arg_size, size_t *first_arg_len_result);

extern time_t init_biologistTimeOut();
extern bool LEVEL_DELTA(int iLevel, int yLevel, int iDifLev);

extern void split_argument(const char *argument, std::vector<std::string> & vecArgs);

extern int CalculateDuration(int iSpd, int iDur);

extern float gauss_random(float avg = 0, float sigma = 1);

extern int parse_time_str(const char* str);
extern bool has_only_digits(const std::string string_name);
extern bool WildCaseCmp(const char *w, const char *s);

std::string num_with_point(int n);

#endif /* __INC_METIN_II_UTILS_H__ */

