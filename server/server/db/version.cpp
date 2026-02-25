#include "stdafx.h"

void WriteVersion()
{
	FILE* fp(fopen("VERSION.txt", "w"));

	if (fp)
	{
		fprintf(fp, "db version: 4.6.2\n");
		fclose(fp);
	}
}


